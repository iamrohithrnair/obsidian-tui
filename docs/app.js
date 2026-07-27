/* obsidian-tui landing page behaviour.
   Three small things: recolour the demo terminal with the app's real theme
   seeds, copy install commands, and draw a graph that behaves like the app's
   (it settles, then stops burning cycles). */

(() => {
  'use strict';

  const $  = (s, r = document) => r.querySelector(s);
  const $$ = (s, r = document) => [...r.querySelectorAll(s)];
  const reduced = window.matchMedia('(prefers-reduced-motion: reduce)').matches;

  /* ── theme seeds, copied verbatim from crates/otui-theme/src/seed.rs ───── */

  const THEMES = [
    { name: 'obsidian-dark',  bg: '#1e1e1e', alt: '#161616', border: '#2f2f2f', active: '#34304a',
      text: '#dadada', muted: '#9a9a9a', faint: '#6b6b6b', accent: '#8b6cef', green: '#44cf6e', blue: '#4d9fff' },
    { name: 'obsidian-light', bg: '#ffffff', alt: '#f8f8f8', border: '#dcddde', active: '#ddd7f5',
      text: '#222222', muted: '#5c5c5c', faint: '#8f8f8f', accent: '#705dcf', green: '#08b94e', blue: '#086ddd' },
    { name: 'catppuccin',     bg: '#1e1e2e', alt: '#181825', border: '#313244', active: '#45475a',
      text: '#cdd6f4', muted: '#a6adc8', faint: '#6c7086', accent: '#cba6f7', green: '#a6e3a1', blue: '#89b4fa' },
    { name: 'tokyo-night',    bg: '#1a1b26', alt: '#16161e', border: '#292e42', active: '#364a82',
      text: '#c0caf5', muted: '#a9b1d6', faint: '#565f89', accent: '#bb9af7', green: '#9ece6a', blue: '#7aa2f7' },
    { name: 'gruvbox',        bg: '#282828', alt: '#1d2021', border: '#3c3836', active: '#504945',
      text: '#ebdbb2', muted: '#bdae93', faint: '#928374', accent: '#d3869b', green: '#b8bb26', blue: '#83a598' },
    { name: 'nord',           bg: '#2e3440', alt: '#292e39', border: '#3b4252', active: '#434c5e',
      text: '#eceff4', muted: '#d8dee9', faint: '#7b88a1', accent: '#88c0d0', green: '#a3be8c', blue: '#81a1c1' },
    { name: 'dracula',        bg: '#282a36', alt: '#21222c', border: '#44475a', active: '#44475a',
      text: '#f8f8f2', muted: '#c8c9d4', faint: '#6272a4', accent: '#bd93f9', green: '#50fa7b', blue: '#8be9fd' },
    { name: 'rosé pine',      bg: '#191724', alt: '#1f1d2e', border: '#26233a', active: '#403d52',
      text: '#e0def4', muted: '#908caa', faint: '#6e6a86', accent: '#c4a7e7', green: '#9ccfd8', blue: '#7ab8d4' },
    { name: 'everforest',     bg: '#2d353b', alt: '#272e33', border: '#3d484d', active: '#475258',
      text: '#d3c6aa', muted: '#9da9a0', faint: '#7a8478', accent: '#a7c080', green: '#a7c080', blue: '#7fbbb3' },
    { name: 'solarized',      bg: '#fdf6e3', alt: '#eee8d5', border: '#d9d2bf', active: '#ded8c5',
      text: '#586e75', muted: '#657b83', faint: '#93a1a1', accent: '#268bd2', green: '#859900', blue: '#268bd2' }
  ];

  /* A light theme needs dark text on the accent-filled status bar. */
  const luminance = (hex) => {
    const n = parseInt(hex.slice(1), 16);
    const f = (c) => { c /= 255; return c <= .03928 ? c / 12.92 : ((c + .055) / 1.055) ** 2.4; };
    return .2126 * f(n >> 16 & 255) + .7152 * f(n >> 8 & 255) + .0722 * f(n & 255);
  };

  function applyTheme(t) {
    const s = document.documentElement.style;
    s.setProperty('--t-bg', t.bg);
    s.setProperty('--t-bg-alt', t.alt);
    s.setProperty('--t-border', t.border);
    s.setProperty('--t-active', t.active);
    s.setProperty('--t-text', t.text);
    s.setProperty('--t-muted', t.muted);
    s.setProperty('--t-faint', t.faint);
    s.setProperty('--t-accent', t.accent);
    s.setProperty('--t-green', t.green);
    s.setProperty('--t-blue', t.blue);
    s.setProperty('--t-on-acc', luminance(t.accent) > 0.45 ? '#101010' : '#ffffff');

    const label = $('#theme-name');
    if (label) label.textContent = t.name;
  }

  const themeBar = $('.themes');
  if (themeBar) {
    THEMES.forEach((t, i) => {
      const b = document.createElement('button');
      b.className = 'chip';
      b.type = 'button';
      b.textContent = t.name;
      b.style.setProperty('--sw', t.accent);
      b.setAttribute('aria-pressed', i === 0 ? 'true' : 'false');
      b.addEventListener('click', () => {
        $$('.chip', themeBar).forEach((c) => c.setAttribute('aria-pressed', 'false'));
        b.setAttribute('aria-pressed', 'true');
        applyTheme(t);
      });
      themeBar.append(b);
    });
  }

  /* ── copy to clipboard ─────────────────────────────────────────────────── */

  const toast = $('#toast');
  let toastTimer;

  function flash(msg) {
    if (!toast) return;
    toast.textContent = msg;
    toast.classList.add('on');
    clearTimeout(toastTimer);
    toastTimer = setTimeout(() => toast.classList.remove('on'), 2000);
  }

  $$('.copy-cmd').forEach((btn) => {
    btn.addEventListener('click', async () => {
      const text = btn.dataset.copy;
      try {
        await navigator.clipboard.writeText(text);
      } catch {
        const ta = document.createElement('textarea');
        ta.value = text;
        ta.style.cssText = 'position:fixed;opacity:0';
        document.body.append(ta);
        ta.select();
        try { document.execCommand('copy'); } catch { /* nothing left to try */ }
        ta.remove();
      }
      btn.classList.add('copied');
      setTimeout(() => btn.classList.remove('copied'), 1600);
      flash('Copied. Paste it into your shell.');
    });
  });

  /* ── install tabs ──────────────────────────────────────────────────────── */

  const tabs = $$('.tab');

  function selectTab(tab) {
    tabs.forEach((t) => {
      const on = t === tab;
      t.classList.toggle('is-on', on);
      t.setAttribute('aria-selected', String(on));
      t.tabIndex = on ? 0 : -1;
      const panel = document.getElementById(t.getAttribute('aria-controls'));
      if (panel) { panel.hidden = !on; panel.classList.toggle('is-on', on); }
    });
  }

  tabs.forEach((tab, i) => {
    tab.addEventListener('click', () => selectTab(tab));
    tab.addEventListener('keydown', (e) => {
      const step = e.key === 'ArrowRight' ? 1 : e.key === 'ArrowLeft' ? -1 : 0;
      if (!step) return;
      e.preventDefault();
      const next = tabs[(i + step + tabs.length) % tabs.length];
      selectTab(next);
      next.focus();
    });
  });

  /* ── nav border on scroll ──────────────────────────────────────────────── */

  const nav = $('#nav');
  const onScroll = () => nav && nav.classList.toggle('stuck', window.scrollY > 8);
  addEventListener('scroll', onScroll, { passive: true });
  onScroll();

  /* ── the recording ─────────────────────────────────────────────────────── */

  /* Half a minute of looping motion is exactly what the OS setting is asking
     about, so leave it on the poster frame and let the controls start it. */
  const film = $('#film-video');
  if (film && reduced) {
    film.removeAttribute('autoplay');
    film.pause();
  }

  /* ── the graph ─────────────────────────────────────────────────────────── */

  const canvas = $('#graph');
  if (canvas && canvas.getContext) drawGraph(canvas);

  function drawGraph(cv) {
    const ctx = cv.getContext('2d');
    const W = 720, H = 460;

    // A small vault: solid nodes exist, hollow ones are only linked to.
    const labels = [
      'Welcome', 'Roadmap', 'Backlog', 'Ideas', 'Daily/2026-07-27', 'Reading',
      'Migration', 'Q3 Planning', 'Meetings', 'People', 'Books', 'Rust',
      'Terminal', 'Vault', 'Graph theory', 'Someday', 'Archive', 'Recipes',
      'Journal', 'Music', 'Papers', 'Snippets'
    ];
    const ghostCount = 6;

    const nodes = labels.map((l, i) => ({
      label: l,
      ghost: i >= labels.length - ghostCount,
      x: W / 2 + (Math.random() - .5) * 260,
      y: H / 2 + (Math.random() - .5) * 200,
      vx: 0, vy: 0, deg: 0
    }));

    const pairs = [
      [0,1],[0,3],[0,4],[1,2],[1,7],[2,7],[3,14],[3,15],[4,8],[4,18],
      [5,10],[5,20],[6,7],[6,1],[8,9],[9,8],[10,5],[11,12],[11,21],[12,13],
      [13,0],[13,14],[14,3],[16,2],[16,17],[17,16],[19,20],[20,5],[21,11],
      [0,13],[7,6],[3,11],[1,16],[4,19]
    ].filter(([a, b]) => a < nodes.length && b < nodes.length && a !== b);

    pairs.forEach(([a, b]) => { nodes[a].deg++; nodes[b].deg++; });

    const radius = (n) => (n.ghost ? 4 : 4.2 + Math.min(n.deg, 7) * 0.85);

    let alpha = 1;            // cooling factor; the sim stops when it settles
    let raf = null;
    let running = false;

    function step() {
      const k = 0.032 * alpha;

      // repulsion (naive, but this graph is 22 nodes)
      for (let i = 0; i < nodes.length; i++) {
        for (let j = i + 1; j < nodes.length; j++) {
          const a = nodes[i], b = nodes[j];
          let dx = b.x - a.x, dy = b.y - a.y;
          let d2 = dx * dx + dy * dy || 0.01;
          const d = Math.sqrt(d2);
          const f = 2600 / d2;
          const fx = (dx / d) * f, fy = (dy / d) * f;
          a.vx -= fx; a.vy -= fy;
          b.vx += fx; b.vy += fy;
        }
      }

      // springs
      for (const [ai, bi] of pairs) {
        const a = nodes[ai], b = nodes[bi];
        const dx = b.x - a.x, dy = b.y - a.y;
        const d = Math.hypot(dx, dy) || 0.01;
        const f = (d - 96) * 0.045;
        const fx = (dx / d) * f, fy = (dy / d) * f;
        a.vx += fx; a.vy += fy;
        b.vx -= fx; b.vy -= fy;
      }

      // gentle pull to the middle, then integrate
      let energy = 0;
      for (const n of nodes) {
        n.vx += (W / 2 - n.x) * 0.0022;
        n.vy += (H / 2 - n.y) * 0.0028;
        n.vx *= 0.86; n.vy *= 0.86;
        n.x += n.vx * k * 30;
        n.y += n.vy * k * 30;
        n.x = Math.max(34, Math.min(W - 34, n.x));
        n.y = Math.max(28, Math.min(H - 28, n.y));
        energy += Math.abs(n.vx) + Math.abs(n.vy);
      }

      alpha *= 0.994;
      return energy / nodes.length;
    }

    function paint() {
      const dpr = Math.min(devicePixelRatio || 1, 2);
      if (cv.width !== W * dpr) { cv.width = W * dpr; cv.height = H * dpr; }
      ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
      ctx.clearRect(0, 0, W, H);

      ctx.lineWidth = 1;
      for (const [ai, bi] of pairs) {
        const a = nodes[ai], b = nodes[bi];
        ctx.strokeStyle = (a.ghost || b.ghost) ? 'rgba(139,108,239,.18)' : 'rgba(160,160,190,.16)';
        ctx.beginPath();
        ctx.moveTo(a.x, a.y);
        ctx.lineTo(b.x, b.y);
        ctx.stroke();
      }

      ctx.font = '11px "JetBrains Mono", ui-monospace, monospace';
      ctx.textAlign = 'center';
      ctx.textBaseline = 'top';

      for (const n of nodes) {
        const r = radius(n);
        ctx.beginPath();
        ctx.arc(n.x, n.y, r, 0, Math.PI * 2);
        if (n.ghost) {
          ctx.strokeStyle = '#a882ff';
          ctx.lineWidth = 1.4;
          ctx.stroke();
        } else {
          ctx.fillStyle = '#a882ff';
          ctx.globalAlpha = 0.55 + Math.min(n.deg, 6) * 0.075;
          ctx.fill();
          ctx.globalAlpha = 1;
        }
        if (n.deg >= 4 || n.ghost) {
          ctx.fillStyle = n.ghost ? 'rgba(168,130,255,.72)' : 'rgba(200,201,215,.62)';
          ctx.fillText(n.label, n.x, n.y + r + 5);
        }
      }
    }

    function loop() {
      const energy = step();
      paint();
      if (energy > 0.06 && alpha > 0.02) {
        raf = requestAnimationFrame(loop);
      } else {
        running = false;
        raf = null;
      }
    }

    function start() {
      if (running) return;
      running = true;
      raf = requestAnimationFrame(loop);
    }

    if (reduced) {
      // Settle it silently, then draw the result once.
      for (let i = 0; i < 400; i++) step();
      paint();
    } else if ('IntersectionObserver' in window) {
      const io = new IntersectionObserver((entries) => {
        for (const e of entries) {
          if (e.isIntersecting) start();
          else if (raf) { cancelAnimationFrame(raf); raf = null; running = false; }
        }
      }, { threshold: 0.15 });
      io.observe(cv);
    } else {
      start();
    }

    // Nudging it back to life is half the fun of a graph view.
    if (!reduced) {
      cv.addEventListener('pointerdown', (e) => {
        const rect = cv.getBoundingClientRect();
        const px = (e.clientX - rect.left) / rect.width * W;
        const py = (e.clientY - rect.top) / rect.height * H;
        for (const n of nodes) {
          const d = Math.hypot(n.x - px, n.y - py) || 1;
          const f = Math.min(700 / (d * d), 9);
          n.vx += (n.x - px) / d * f;
          n.vy += (n.y - py) / d * f;
        }
        alpha = Math.max(alpha, 0.6);
        start();
      });
      cv.style.cursor = 'grab';
    }
  }
})();
