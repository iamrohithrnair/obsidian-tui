//! Drawing pictures in the terminal.
//!
//! Terminals that speak Kitty's graphics protocol, iTerm2's, or sixel can show
//! real pixels; the rest get half-block mosaics, which are coarse but always
//! work. [`ratatui_image`] handles the encoding, and [`Picker`] asks the
//! terminal which of those it supports. That question has to be asked *before*
//! the alternate screen is entered, because it is answered on stdin.
//!
//! Two things make this awkward in a scrolling reading pane, and both are
//! handled here:
//!
//! * Encoding an image takes long enough to drop frames, so it happens on a
//!   worker thread and the pane draws a placeholder until it lands.
//! * The layout has to know how tall a picture is *before* it is decoded, or
//!   the text below it would jump when it arrives. Image headers carry the
//!   pixel size and are cheap to read, so the height is known from the first
//!   frame and the picture fades in without moving anything.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, Sender, TryRecvError};

use ratatui::layout::Size;
use ratatui_image::picker::Picker;
use ratatui_image::sliced::SlicedProtocol;
use ratatui_image::{FontSize, Resize};

/// Extensions worth opening. Anything else — a PDF, an embedded note — is left
/// to the text fallback rather than handed to a decoder that will refuse it.
const EXTENSIONS: &[&str] = &["png", "jpg", "jpeg", "gif", "webp", "bmp"];

/// How many encoded images to hold on to.
///
/// Every width gets its own encoding, so dragging a terminal edge from 80 to
/// 200 columns walks through a hundred of them. Without a cap that is a hundred
/// copies of the picture sitting in memory.
const MAX_CACHED: usize = 24;

/// Whether a path is worth trying to draw.
#[must_use]
pub fn is_image(path: &Path) -> bool {
    path.extension()
        .and_then(|ext| ext.to_str())
        .is_some_and(|ext| EXTENSIONS.contains(&ext.to_ascii_lowercase().as_str()))
}

/// The terminal's picture-drawing ability, plus everything already drawn.
pub struct Images {
    /// `None` when images are switched off, or the terminal was never asked.
    picker: Option<Picker>,
    /// Ceiling on how tall one picture may be drawn, in terminal rows.
    max_rows: u16,
    /// Pixel dimensions by path. `None` records a file that could not be read,
    /// so a missing image is not re-opened on every frame.
    dims: HashMap<PathBuf, Option<(u32, u32)>>,
    cache: HashMap<Key, Entry>,
    clock: u64,
    worker: Option<Worker>,
}

type Key = (PathBuf, u16, u16);

struct Entry {
    state: State,
    used: u64,
}

enum State {
    Pending,
    Ready(Box<SlicedProtocol>),
    Failed,
}

struct Worker {
    jobs: Sender<Job>,
    done: Receiver<(Key, Option<Box<SlicedProtocol>>)>,
}

struct Job {
    key: Key,
    picker: Picker,
}

impl Images {
    /// Asks the terminal what it can draw.
    ///
    /// Must be called before the alternate screen is entered: the query is
    /// written to stdout and the answer read back from stdin, which only works
    /// while the terminal is still in its normal mode.
    ///
    /// A terminal that doesn't answer gets half-blocks, which need no support
    /// at all. `max_rows` of 0, or a terminal that can't even do that, switches
    /// images off and leaves the alt text in their place.
    #[must_use]
    pub fn probe(enabled: bool, max_rows: u16) -> Self {
        let picker = (enabled && max_rows > 0)
            .then(Picker::from_query_stdio)
            .and_then(Result::ok);
        Self::with_picker(picker, max_rows)
    }

    /// An `Images` that draws nothing, for tests and headless runs.
    #[must_use]
    pub fn disabled() -> Self {
        Self::with_picker(None, 0)
    }

    /// An `Images` with no terminal behind it, for tests.
    ///
    /// Half-blocks assume a 10x20 cell, which is what the tests measure
    /// against.
    #[cfg(test)]
    #[must_use]
    pub fn halfblocks(max_rows: u16) -> Self {
        Self::with_picker(Some(Picker::halfblocks()), max_rows)
    }

    fn with_picker(picker: Option<Picker>, max_rows: u16) -> Self {
        Self {
            picker,
            max_rows,
            dims: HashMap::new(),
            cache: HashMap::new(),
            clock: 0,
            worker: None,
        }
    }

    /// Whether anything can be drawn at all.
    #[must_use]
    pub fn enabled(&self) -> bool {
        self.picker.is_some()
    }

    /// How much room a picture will take, in terminal cells.
    ///
    /// Read from the file's header rather than a decoded image, so the answer
    /// is available on the first frame and the layout never has to move once
    /// the picture arrives. `None` means there is nothing to draw and the
    /// caller should fall back to text.
    pub fn measure(&mut self, path: &Path, available: Size) -> Option<Size> {
        let picker = self.picker.as_ref()?;
        if available.width == 0 || !is_image(path) {
            return None;
        }

        let pixels = *self
            .dims
            .entry(path.to_path_buf())
            .or_insert_with(|| read_dimensions(path));
        let pixels = pixels?;

        let room = Size::new(available.width, available.height.min(self.max_rows));
        Some(fit(pixels, picker.font_size(), room))
    }

    /// How many cells wide a pixel width is, for Obsidian's `![[x.png|400]]`.
    #[must_use]
    pub fn cells_wide(&self, pixels: u32) -> Option<u16> {
        let font = self.picker.as_ref()?.font_size();
        let cells = pixels / u32::from(font.width.max(1));
        Some(u16::try_from(cells).unwrap_or(u16::MAX).max(1))
    }

    /// The encoded picture, if it is ready.
    ///
    /// A miss starts the work and returns `None`; the next frame after it lands
    /// will get it. Callers should draw a placeholder in the space [`measure`]
    /// reserved.
    ///
    /// [`measure`]: Self::measure
    pub fn get(&mut self, path: &Path, size: Size) -> Option<&SlicedProtocol> {
        let picker = self.picker.as_ref()?.clone();
        let key = (path.to_path_buf(), size.width, size.height);
        self.clock += 1;

        if !self.cache.contains_key(&key) {
            self.evict();
            self.cache.insert(
                key.clone(),
                Entry {
                    state: State::Pending,
                    used: self.clock,
                },
            );
            let worker = self.worker.get_or_insert_with(spawn_worker);
            // A dead worker leaves the entry pending forever, which draws the
            // placeholder rather than crashing the reader.
            let _ = worker.jobs.send(Job {
                key: key.clone(),
                picker,
            });
        }

        let entry = self.cache.get_mut(&key)?;
        entry.used = self.clock;
        match &entry.state {
            State::Ready(protocol) => Some(protocol),
            State::Pending | State::Failed => None,
        }
    }

    /// Takes in finished work. Returns whether anything new arrived.
    pub fn poll(&mut self) -> bool {
        let Some(worker) = &self.worker else {
            return false;
        };
        let mut arrived = false;
        loop {
            match worker.done.try_recv() {
                Ok((key, protocol)) => {
                    if let Some(entry) = self.cache.get_mut(&key) {
                        entry.state = match protocol {
                            Some(protocol) => State::Ready(protocol),
                            None => State::Failed,
                        };
                        arrived = true;
                    }
                }
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => {
                    self.worker = None;
                    break;
                }
            }
        }
        arrived
    }

    /// Drops everything remembered, so edited files are read again.
    pub fn forget(&mut self) {
        self.dims.clear();
        self.cache.clear();
    }

    /// Drops the least recently drawn half once the cache is full.
    ///
    /// Half at a time rather than one at a time, so a resize that invalidates
    /// every entry doesn't evict on every single frame.
    fn evict(&mut self) {
        if self.cache.len() < MAX_CACHED {
            return;
        }
        let mut used: Vec<u64> = self.cache.values().map(|entry| entry.used).collect();
        used.sort_unstable();
        let cutoff = used[used.len() / 2];
        self.cache.retain(|_, entry| entry.used > cutoff);
    }
}

/// Scales `pixels` to fit `available`, in cells.
///
/// Pictures are never scaled up: a 16-pixel icon stays an icon rather than
/// being blown across the pane.
fn fit(pixels: (u32, u32), font: FontSize, available: Size) -> Size {
    let (width, height) = pixels;
    if width == 0 || height == 0 {
        return Size::new(0, 0);
    }

    let room = |cells: u16, per_cell: u16| f64::from(cells) * f64::from(per_cell.max(1));
    let scale = (room(available.width, font.width) / f64::from(width))
        .min(room(available.height, font.height) / f64::from(height))
        .min(1.0);

    let cells = |pixels: u32, per_cell: u16, limit: u16| {
        let cells = (f64::from(pixels) * scale / f64::from(per_cell.max(1))).ceil();
        (cells as u16).clamp(1, limit.max(1))
    };
    Size::new(
        cells(width, font.width, available.width),
        cells(height, font.height, available.height),
    )
}

/// Reads an image's pixel size without decoding it.
fn read_dimensions(path: &Path) -> Option<(u32, u32)> {
    image::ImageReader::open(path)
        .ok()?
        .with_guessed_format()
        .ok()?
        .into_dimensions()
        .ok()
}

/// Runs decoding and encoding away from the draw loop.
///
/// One thread, so a note full of pictures fills in top to bottom rather than
/// starting every decode at once and finishing none of them.
fn spawn_worker() -> Worker {
    let (jobs, rx) = mpsc::channel::<Job>();
    let (tx, done) = mpsc::channel();

    std::thread::Builder::new()
        .name("otui-images".into())
        .spawn(move || {
            for job in rx {
                let (path, width, height) = &job.key;
                let size = Size::new(*width, *height);
                let protocol = image::ImageReader::open(path)
                    .ok()
                    .and_then(|reader| reader.with_guessed_format().ok())
                    .and_then(|reader| reader.decode().ok())
                    .and_then(|image| {
                        SlicedProtocol::new_with_resize(&job.picker, image, size, Resize::Fit(None))
                            .ok()
                    });
                if tx.send((job.key, protocol.map(Box::new))).is_err() {
                    return;
                }
            }
        })
        // A machine that cannot spawn a thread has bigger problems than
        // pictures; the channel simply stays empty and placeholders remain.
        .ok();

    Worker { jobs, done }
}

#[cfg(test)]
mod tests {
    use super::*;

    const FONT: FontSize = FontSize {
        width: 10,
        height: 20,
    };

    #[test]
    fn a_wide_picture_is_bounded_by_the_pane_width() {
        // 1000x200 pixels is 100x10 cells at this font, wider than the pane.
        let size = fit((1000, 200), FONT, Size::new(40, 30));
        assert_eq!(size.width, 40, "fills the width it is given");
        assert_eq!(size.height, 4, "and keeps its shape: 200 * 0.4 / 20");
    }

    #[test]
    fn a_tall_picture_is_bounded_by_the_row_cap() {
        let size = fit((200, 1000), FONT, Size::new(40, 10));
        assert_eq!(size.height, 10, "clipped to the rows allowed");
        assert_eq!(size.width, 4, "200 * 0.2 / 10");
    }

    #[test]
    fn a_small_picture_is_left_alone() {
        let size = fit((100, 40), FONT, Size::new(80, 20));
        assert_eq!(
            (size.width, size.height),
            (10, 2),
            "an icon stays an icon rather than being blown up to fill the pane"
        );
    }

    #[test]
    fn a_picture_always_gets_at_least_one_cell() {
        let size = fit((3, 3), FONT, Size::new(80, 20));
        assert_eq!(
            (size.width, size.height),
            (1, 1),
            "never rounds away to nothing"
        );
    }

    #[test]
    fn only_decodable_files_are_offered_to_the_decoder() {
        assert!(
            is_image(Path::new("a/b/chart.PNG")),
            "extensions vary in case"
        );
        assert!(is_image(Path::new("chart.jpeg")));
        assert!(
            !is_image(Path::new("notes/other.md")),
            "an embedded note is not a picture"
        );
        assert!(!is_image(Path::new("paper.pdf")));
        assert!(!is_image(Path::new("no-extension")));
    }

    #[test]
    fn a_disabled_terminal_measures_nothing() {
        let mut images = Images::disabled();
        assert!(!images.enabled());
        assert_eq!(
            images.measure(Path::new("chart.png"), Size::new(80, 20)),
            None
        );
        assert!(!images.poll(), "and never has work to collect");
    }
}
