//! The updater's only way out: a GET over HTTPS to hosts it names, as the
//! Android and Mac updaters do. Redirects are followed one at a time, each
//! checked (GitHub hands a release's file from github.com to
//! release-assets.githubusercontent.com); at most a given size, read as it
//! comes so a download can be hashed as it's written.

use std::io::Read;
use std::time::{Duration, Instant};

use ureq::http::Uri;
use ureq::tls::{RootCerts, TlsConfig};

/// Where checking and downloading may go.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hosts {
    pub names: &'static [&'static str],
    /// Plain HTTP to this computer (127.0.0.1), for a test's stand-in for
    /// GitHub: only test builds' test sources set it.
    pub loopback: bool,
}

/// GitHub's API, where checking asks.
pub const API: Hosts = Hosts {
    names: &["api.github.com"],
    loopback: false,
};

/// github.com, and where it hands a release's files.
pub const DOWNLOADS: Hosts = Hosts {
    names: &[
        "github.com",
        "objects.githubusercontent.com",
        "release-assets.githubusercontent.com",
    ],
    loopback: false,
};

/// A test's stand-in on this computer.
pub const TEST: Hosts = Hosts {
    names: &[],
    loopback: true,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FetchError {
    /// Somewhere it may not go: not HTTPS, or not one of its hosts.
    Refused(String),
    /// Longer than it may be.
    TooLong,
    TooSlow,
    /// Nothing there (404).
    NotFound,
    Failed(String),
}

impl std::fmt::Display for FetchError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            FetchError::Refused(why) => write!(f, "refused: {why}"),
            FetchError::TooLong => write!(f, "longer than it may be"),
            FetchError::TooSlow => write!(f, "too slow"),
            FetchError::NotFound => write!(f, "not there"),
            FetchError::Failed(why) => write!(f, "{why}"),
        }
    }
}

/// Whether a request may start at, be redirected to, or end at `url`.
pub fn permits(url: &str, hosts: &Hosts) -> bool {
    let Ok(uri) = url.parse::<Uri>() else {
        return false;
    };
    let (Some(scheme), Some(host)) = (uri.scheme_str(), uri.host()) else {
        return false;
    };
    // No user name or password in it.
    if uri.authority().is_some_and(|a| a.as_str().contains('@')) {
        return false;
    }
    if hosts.loopback && host == "127.0.0.1" && (scheme == "http" || scheme == "https") {
        return true;
    }
    scheme == "https"
        && uri.port_u16().is_none_or(|port| port == 443)
        && hosts
            .names
            .iter()
            .any(|name| host.eq_ignore_ascii_case(name))
}

const MAX_REDIRECTS: usize = 5;

/// GETs `url`, at most `max` bytes within `seconds`, following redirects
/// only to where `hosts` permits. `started` hears the length the server
/// says (when it says one); `sink` gets the body as it comes.
pub fn get(
    url: &str,
    hosts: &Hosts,
    max: u64,
    seconds: u64,
    mut started: impl FnMut(Option<u64>),
    mut sink: impl FnMut(&[u8]) -> std::io::Result<()>,
) -> Result<(), FetchError> {
    let deadline = Instant::now() + Duration::from_secs(seconds.max(1));
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .http_status_as_error(false)
        .max_redirects(0)
        .https_only(!hosts.loopback)
        .timeout_connect(Some(Duration::from_secs(15)))
        .timeout_global(Some(Duration::from_secs(seconds.max(1) + 5)))
        .user_agent("PassBundle")
        .tls_config(
            TlsConfig::builder()
                .root_certs(RootCerts::PlatformVerifier)
                .build(),
        )
        .build()
        .into();
    let mut at = url.to_owned();
    for _ in 0..=MAX_REDIRECTS {
        if !permits(&at, hosts) {
            return Err(FetchError::Refused(format!("not one of its hosts: {at}")));
        }
        let mut request = agent.get(&at);
        if at.starts_with("https://api.github.com/") {
            request = request
                .header("Accept", "application/vnd.github+json")
                .header("X-GitHub-Api-Version", "2022-11-28");
        }
        let mut response = request.call().map_err(|e| match e {
            ureq::Error::Timeout(_) => FetchError::TooSlow,
            e => FetchError::Failed(e.to_string()),
        })?;
        let status = response.status().as_u16();
        if matches!(status, 301 | 302 | 303 | 307 | 308) {
            let Some(next) = response
                .headers()
                .get("location")
                .and_then(|v| v.to_str().ok())
            else {
                return Err(FetchError::Failed("a redirect to nowhere".into()));
            };
            // Only to a whole address: one this checks, as it is.
            if !next.starts_with("https://") && !next.starts_with("http://") {
                return Err(FetchError::Refused(format!(
                    "redirected somewhere unexpected: {next}"
                )));
            }
            at = next.to_owned();
            continue;
        }
        if status == 404 {
            return Err(FetchError::NotFound);
        }
        if !(200..300).contains(&status) {
            return Err(FetchError::Failed(format!("HTTP {status}")));
        }
        let length = response
            .headers()
            .get("content-length")
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.trim().parse::<u64>().ok());
        if length.is_some_and(|n| n > max) {
            return Err(FetchError::TooLong);
        }
        started(length);
        // One byte past the limit, so a longer body is caught here rather
        // than taken for a short one.
        let mut reader = response.body_mut().with_config().limit(max + 1).reader();
        let mut buffer = vec![0u8; 64 << 10];
        let mut read: u64 = 0;
        loop {
            if Instant::now() > deadline {
                return Err(FetchError::TooSlow);
            }
            let n = match reader.read(&mut buffer) {
                Ok(0) => break,
                Ok(n) => n,
                Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
                Err(e) if e.kind() == std::io::ErrorKind::TimedOut => {
                    return Err(FetchError::TooSlow)
                }
                Err(e) => {
                    // ureq's own limit, a byte past ours.
                    if read >= max {
                        return Err(FetchError::TooLong);
                    }
                    return Err(FetchError::Failed(e.to_string()));
                }
            };
            read += n as u64;
            if read > max {
                return Err(FetchError::TooLong);
            }
            sink(&buffer[..n]).map_err(|e| FetchError::Failed(format!("keeping it: {e}")))?;
        }
        return Ok(());
    }
    Err(FetchError::Failed("too many redirects".into()))
}

/// A small file, whole.
pub fn get_bytes(url: &str, hosts: &Hosts, max: u64, seconds: u64) -> Result<Vec<u8>, FetchError> {
    let mut body = Vec::new();
    get(
        url,
        hosts,
        max,
        seconds,
        |_| {},
        |chunk| {
            body.extend_from_slice(chunk);
            Ok(())
        },
    )?;
    Ok(body)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_https_to_its_hosts() {
        assert!(permits("https://api.github.com/repos/x", &API));
        assert!(permits("https://API.GitHub.com/repos/x", &API));
        assert!(permits(
            "https://release-assets.githubusercontent.com/github-production-release-asset/1/2",
            &DOWNLOADS
        ));
        for url in [
            "http://api.github.com/repos/x",
            "https://api.github.com.evil.example/repos/x",
            "https://evil.example/api.github.com",
            "https://user@api.github.com/repos/x",
            "https://api.github.com:8443/repos/x",
            "ftp://api.github.com/x",
            "file:///etc/passwd",
            "/repos/x",
            "",
        ] {
            assert!(!permits(url, &API), "{url}");
        }
        // The API's host isn't a download host, nor the other way around.
        assert!(!permits("https://github.com/x", &API));
        assert!(!permits("https://api.github.com/x", &DOWNLOADS));
        // A test's stand-in: this computer only, and nothing else.
        assert!(permits("http://127.0.0.1:8123/repos/x", &TEST));
        assert!(!permits("http://localhost:8123/repos/x", &TEST));
        assert!(!permits("https://api.github.com/repos/x", &TEST));
        assert!(!permits("http://127.0.0.1:8123/repos/x", &API));
    }

    /// A path, and its status, headers, and body.
    type Route = (&'static str, u16, Vec<(&'static str, String)>, Vec<u8>);

    /// A server on this computer, answering each request from `routes`
    /// until `count` have been answered.
    fn serve(routes: Vec<Route>, count: usize) -> (String, std::thread::JoinHandle<Vec<String>>) {
        let server = tiny_http::Server::http("127.0.0.1:0").unwrap();
        let base = format!("http://{}", server.server_addr().to_ip().unwrap());
        let handle = std::thread::spawn(move || {
            let mut seen = Vec::new();
            for _ in 0..count {
                let request = server.recv().unwrap();
                seen.push(request.url().to_owned());
                let route = routes.iter().find(|r| r.0 == request.url());
                let (status, headers, body) = match route {
                    Some((_, status, headers, body)) => (*status, headers.clone(), body.clone()),
                    None => (404, vec![], vec![]),
                };
                // "x-chunked": no length given, as a server may send it.
                let chunked = headers.iter().any(|(name, _)| *name == "x-chunked");
                let length = (!chunked).then_some(body.len());
                let headers = headers
                    .iter()
                    .filter(|(name, _)| *name != "x-chunked")
                    .map(|(name, value)| {
                        tiny_http::Header::from_bytes(name.as_bytes(), value.as_bytes()).unwrap()
                    })
                    .collect();
                let response = tiny_http::Response::new(
                    tiny_http::StatusCode(status),
                    headers,
                    std::io::Cursor::new(body),
                    length,
                    None,
                );
                let _ = request.respond(response);
            }
            seen
        });
        (base, handle)
    }

    #[test]
    fn redirects_are_followed_only_where_it_may_go() {
        let (base, server) = serve(
            vec![
                ("/a", 302, vec![("Location", String::new())], vec![]),
                ("/b", 200, vec![], b"the file".to_vec()),
            ],
            2,
        );
        // The redirect's address, now that the port is known.
        let (base2, server2) = serve(
            vec![
                ("/a", 302, vec![("Location", format!("{base}/b"))], vec![]),
                (
                    "/away",
                    302,
                    vec![("Location", "https://evil.example/x".into())],
                    vec![],
                ),
            ],
            2,
        );
        assert_eq!(
            get_bytes(&format!("{base2}/a"), &TEST, 100, 10).unwrap(),
            b"the file"
        );
        assert!(matches!(
            get_bytes(&format!("{base2}/away"), &TEST, 100, 10),
            Err(FetchError::Refused(_))
        ));
        server2.join().unwrap();
        // The first server's /a, a redirect with no address.
        assert!(get_bytes(&format!("{base}/a"), &TEST, 100, 10).is_err());
        server.join().unwrap();
    }

    #[test]
    fn a_body_longer_than_allowed_is_refused_whatever_it_says() {
        let (base, server) = serve(
            vec![
                ("/says", 200, vec![], vec![7u8; 1000]),
                (
                    "/quiet",
                    200,
                    vec![("x-chunked", String::new())],
                    vec![7u8; 100_000],
                ),
                ("/404", 404, vec![], vec![]),
            ],
            5,
        );
        assert_eq!(
            get_bytes(&format!("{base}/says"), &TEST, 999, 10),
            Err(FetchError::TooLong)
        );
        assert_eq!(
            get_bytes(&format!("{base}/says"), &TEST, 1000, 10)
                .unwrap()
                .len(),
            1000
        );
        // Saying nothing of its length, it's stopped at the limit.
        assert_eq!(
            get_bytes(&format!("{base}/quiet"), &TEST, 99_999, 10),
            Err(FetchError::TooLong)
        );
        assert_eq!(
            get_bytes(&format!("{base}/quiet"), &TEST, 100_000, 10)
                .unwrap()
                .len(),
            100_000
        );
        assert_eq!(
            get_bytes(&format!("{base}/404"), &TEST, 1000, 10),
            Err(FetchError::NotFound)
        );
        server.join().unwrap();
    }
}
