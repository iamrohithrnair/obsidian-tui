//! The HTTP client, and how it is told to trust a corporate network.
//!
//! A managed network usually terminates TLS at a proxy and re-signs it with the
//! company's own certificate authority. That CA is in the machine's trust store,
//! put there by whoever set the laptop up — but not in the Mozilla root list this
//! binary ships with, so every request fails with "unknown issuer" and no clue as
//! to why.
//!
//! Every other tool solves this the same way, by reading a CA bundle named in the
//! environment: `SSL_CERT_FILE` for anything built on OpenSSL, `CURL_CA_BUNDLE`,
//! `REQUESTS_CA_BUNDLE`, `NODE_EXTRA_CA_CERTS`. On a machine set up for this kind
//! of network one of those is nearly always already exported, so honouring them
//! means the assistant works with nothing to configure.
//!
//! Proxies need no work at all: [`ureq`] reads `HTTPS_PROXY` and `NO_PROXY` on its
//! own. They are reported here anyway, because "which proxy did it use" is the
//! other half of the same diagnosis.

use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use ureq::Agent;
use ureq::tls::{Certificate, RootCerts, TlsConfig};

/// Variables that may name a CA bundle, in the order they are tried.
///
/// This app's own first, so it can differ from the rest of the toolchain, then
/// the conventional ones from most to least widely honoured.
const CA_BUNDLE_VARS: &[&str] = &[
    "OTUI_CA_BUNDLE",
    "SSL_CERT_FILE",
    "REQUESTS_CA_BUNDLE",
    "CURL_CA_BUNDLE",
    "NODE_EXTRA_CA_CERTS",
    "CARGO_HTTP_CAINFO",
];

/// A directory of certificates, OpenSSL's other convention.
const CA_DIR_VAR: &str = "SSL_CERT_DIR";

/// Where the roots being trusted came from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Roots {
    /// A bundle named by an environment variable, or configured outright.
    Bundle {
        /// The variable it was named by, or `"config"` if set directly.
        source: String,
        path: PathBuf,
        /// How many certificates were read out of it.
        count: usize,
    },
    /// A bundle was named but could not be used, so the built-in roots are in
    /// play. Carries why, because this is the case worth telling someone about.
    Unusable { source: String, reason: String },
    /// The Mozilla roots compiled into this binary.
    Bundled,
}

impl fmt::Display for Roots {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Bundle {
                source,
                path,
                count,
            } => write!(f, "{count} from ${source} ({})", path.display()),
            Self::Unusable { source, reason } => write!(f, "built-in (${source}: {reason})"),
            Self::Bundled => write!(f, "built-in"),
        }
    }
}

/// What the client is doing about trust and proxies, for `/status` to report.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Trust {
    pub roots: Roots,
    /// The proxy in use, host and port only — a proxy URL may carry a password.
    pub proxy: Option<String>,
}

impl fmt::Display for Trust {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "roots {}", self.roots)?;
        match &self.proxy {
            Some(proxy) => write!(f, ", via {proxy}"),
            None => write!(f, ", no proxy"),
        }
    }
}

static AGENT: OnceLock<(Agent, Trust)> = OnceLock::new();

/// The shared client.
///
/// Built once. Beyond keeping the trust settings in one place, this is what lets
/// connections be reused: a turn with several tool calls talks to the same host
/// repeatedly, and a fresh TLS handshake each time is the slowest part of it.
pub fn agent() -> &'static Agent {
    &built().0
}

/// What the client ended up trusting.
pub fn trust() -> &'static Trust {
    &built().1
}

fn built() -> &'static (Agent, Trust) {
    AGENT.get_or_init(build)
}

/// Builds the client from whatever the environment says about trust.
fn build() -> (Agent, Trust) {
    let (roots, certs) = match named_bundle() {
        Some((source, path)) => classify(&source, path),
        None => (Roots::Bundled, None),
    };

    let mut config = Agent::config_builder();
    if let Some(certs) = certs {
        config = config.tls_config(
            TlsConfig::builder()
                .root_certs(RootCerts::from(certs))
                .build(),
        );
    }

    let config = config.build();
    let proxy = config
        .proxy()
        .map(|proxy| format!("{}:{}", proxy.host(), proxy.port()));

    (Agent::new_with_config(config), Trust { roots, proxy })
}

/// The first CA bundle the environment names, if any.
fn named_bundle() -> Option<(String, PathBuf)> {
    let named = |var: &str| {
        let value = std::env::var(var).ok()?;
        let value = value.trim();
        (!value.is_empty()).then(|| (var.to_string(), PathBuf::from(value)))
    };
    CA_BUNDLE_VARS
        .iter()
        .find_map(|var| named(var))
        .or_else(|| named(CA_DIR_VAR))
}

/// Reads a named bundle, reporting what will be trusted as a result.
///
/// The certificates come back alongside so the file is read once. `None` means the
/// built-in roots stay in play: a bundle that can't be used must not take the app
/// from "can't verify this proxy" to "can't reach anything".
fn classify(source: &str, path: PathBuf) -> (Roots, Option<Vec<Certificate<'static>>>) {
    match read_bundle(&path) {
        Ok(certs) if certs.is_empty() => (
            Roots::Unusable {
                source: source.to_string(),
                reason: "no certificates in it".to_string(),
            },
            None,
        ),
        Ok(certs) => (
            Roots::Bundle {
                source: source.to_string(),
                path,
                count: certs.len(),
            },
            Some(certs),
        ),
        Err(reason) => (
            Roots::Unusable {
                source: source.to_string(),
                reason,
            },
            None,
        ),
    }
}

/// Reads every certificate in a PEM file, or in a directory of them.
///
/// The bundle replaces the built-in roots rather than adding to them, which is
/// what `SSL_CERT_FILE` means everywhere else. An IT-provided bundle is normally
/// the full public set with the company's CA appended, so that is the right
/// reading — but it does mean a file holding *only* a corporate CA will not verify
/// hosts the proxy passes straight through.
fn read_bundle(path: &Path) -> Result<Vec<Certificate<'static>>, String> {
    let files = if path.is_dir() {
        let mut files: Vec<PathBuf> = std::fs::read_dir(path)
            .map_err(|err| err.to_string())?
            .filter_map(|entry| entry.ok().map(|entry| entry.path()))
            .filter(|path| path.is_file())
            .collect();
        // Directory order is arbitrary; a stable read makes the certificate count
        // reproducible, which matters when someone is comparing two machines.
        files.sort();
        files
    } else {
        vec![path.to_path_buf()]
    };

    let mut certs = Vec::new();
    for file in &files {
        let pem = std::fs::read(file).map_err(|err| err.to_string())?;
        // A directory of certs may hold keys and hash symlinks too, so anything
        // that isn't a certificate is skipped rather than failing the lot.
        for item in ureq::tls::parse_pem(&pem).flatten() {
            if let ureq::tls::PemItem::Certificate(cert) = item {
                certs.push(cert);
            }
        }
    }
    Ok(certs)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A certificate section, as a bundle file holds them.
    ///
    /// The bytes inside are not a real certificate, and do not need to be:
    /// [`read_bundle`] finds the certificate sections in a file and skips
    /// everything else, and rustls is what decides at handshake time whether what
    /// they contain is any good.
    const PEM: &str =
        "-----BEGIN CERTIFICATE-----\nbm90IGEgcmVhbCBjZXJ0\n-----END CERTIFICATE-----\n";

    /// A private key section, which a bundle may also hold and which is not a
    /// certificate.
    const KEY: &str =
        "-----BEGIN PRIVATE KEY-----\nbm90IGEgcmVhbCBrZXk=\n-----END PRIVATE KEY-----\n";

    fn dir(tag: &str) -> PathBuf {
        let path = std::env::temp_dir().join(format!("otui-http-{tag}-{}", std::process::id()));
        // A leftover from an earlier run that happened to get the same pid would
        // add certificates the directory test then counts.
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).expect("temp dir");
        path
    }

    #[test]
    fn a_bundle_of_one_certificate_is_read_and_used() {
        let dir = dir("one");
        let path = dir.join("corp.pem");
        std::fs::write(&path, PEM).expect("written");

        let (roots, certs) = classify("SSL_CERT_FILE", path.clone());
        assert_eq!(
            roots,
            Roots::Bundle {
                source: "SSL_CERT_FILE".into(),
                path,
                count: 1
            }
        );
        assert_eq!(
            certs.map(|certs| certs.len()),
            Some(1),
            "and the certificates come back, so the file is read only once"
        );
    }

    #[test]
    fn a_directory_of_certificates_is_read_as_one_bundle() {
        let dir = dir("many");
        for name in ["a.pem", "b.pem"] {
            std::fs::write(dir.join(name), PEM).expect("written");
        }
        // What a hashed OpenSSL directory also holds, and what must not break it.
        std::fs::write(dir.join("README"), "not a certificate\n").expect("written");
        std::fs::write(dir.join("private.pem"), KEY).expect("written");

        let certs = read_bundle(&dir).expect("read");
        assert_eq!(
            certs.len(),
            2,
            "the two certificates only: a key and a stray file are skipped rather \
             than failing the whole directory"
        );
    }

    #[test]
    fn a_bundle_holding_both_a_key_and_a_certificate_yields_the_certificate() {
        let dir = dir("mixed");
        let path = dir.join("both.pem");
        std::fs::write(&path, format!("{KEY}{PEM}")).expect("written");
        assert_eq!(read_bundle(&path).expect("read").len(), 1);
    }

    #[test]
    fn a_bundle_that_cannot_be_used_leaves_the_built_in_roots_alone() {
        let dir = dir("bad");
        let empty = dir.join("empty.pem");
        std::fs::write(&empty, "").expect("written");

        for path in [empty, dir.join("absent.pem")] {
            let (roots, certs) = classify("SSL_CERT_FILE", path);
            assert!(
                matches!(roots, Roots::Unusable { .. }),
                "a file with no certificates in it is not a bundle"
            );
            // The distinction that matters: fall back rather than trust nothing,
            // which would turn a proxy problem into a total outage.
            assert!(certs.is_none(), "and nothing replaces the built-in roots");
        }
    }

    #[test]
    fn with_nothing_configured_the_built_in_roots_are_used() {
        // Cannot assert on the environment of the machine running this, so this
        // checks the reporting rather than the resolution.
        assert_eq!(Roots::Bundled.to_string(), "built-in");
        assert_eq!(
            Roots::Unusable {
                source: "SSL_CERT_FILE".into(),
                reason: "no certificates in it".into()
            }
            .to_string(),
            "built-in ($SSL_CERT_FILE: no certificates in it)",
            "the report has to name the variable, or there is nothing to go and fix"
        );
    }

    #[test]
    fn the_report_says_enough_to_diagnose_a_managed_network() {
        let trust = Trust {
            roots: Roots::Bundle {
                source: "SSL_CERT_FILE".into(),
                path: PathBuf::from("/etc/ssl/corp.pem"),
                count: 142,
            },
            proxy: Some("proxy.corp:8080".into()),
        };
        assert_eq!(
            trust.to_string(),
            "roots 142 from $SSL_CERT_FILE (/etc/ssl/corp.pem), via proxy.corp:8080"
        );
        assert_eq!(
            Trust {
                roots: Roots::Bundled,
                proxy: None
            }
            .to_string(),
            "roots built-in, no proxy"
        );
    }

    #[test]
    fn the_shared_client_is_built_once() {
        assert!(
            std::ptr::eq(agent(), agent()),
            "a second client would mean a second connection pool"
        );
        // Whatever this machine's environment says, the report must be printable.
        assert!(!trust().to_string().is_empty());
    }
}
