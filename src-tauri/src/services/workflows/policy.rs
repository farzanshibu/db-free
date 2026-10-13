// SOT: workflow-policy, workflow-file-path-policy, workflow-http-target-policy

use crate::error::{AppError, AppResult};
use crate::integrations::http::PinnedTarget;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use std::path::{Component, Path, PathBuf};

// WHAT:  The rules that decide which files a workflow may read or write, and which
//        addresses its HTTP nodes may call.
// WHY:   A workflow can come from someone else. Without these rules a Read file node
//        could load ~/.ssh/id_rsa into rows and an HTTP node could send them out, or
//        reach cloud metadata (169.254.169.254) and services bound to localhost.
// HOW:   File paths are canonicalised first, so `..`, symlinks and case tricks resolve
//        before anything is compared, then matched component by component against a
//        deny list. HTTP targets must be http(s) and resolve only to public addresses.
//        The vetted addresses are pinned on the request so a second DNS answer cannot
//        swap in a private one, and redirects are not followed (their targets are
//        not vetted).
// WHERE: nodes/io.rs (Read file, Save file, HTTP nodes); integrations/http.rs (pinning)

// Keep in step with `identifier` in src-tauri/tauri.conf.json: the app's data
// directory is named after it on every platform.
const APP_IDENTIFIER: &str = "app.dbfree.desktop";

// File names the app keeps for itself, wherever they sit.
const APP_OWN_FILE_PREFIXES: &[&str] = &["db-free.sqlite", "master.key"];

// Where the user's home directory keeps credentials, relative to home.
fn home_denied() -> Vec<(String, &'static str)> {
    vec![
        (".ssh".into(), "SSH keys"),
        (".gnupg".into(), "GPG keys"),
        (".aws".into(), "AWS credentials"),
        (".azure".into(), "Azure credentials"),
        (".kube".into(), "Kubernetes credentials"),
        (".docker".into(), "Docker credentials"),
        (".config/gcloud".into(), "Google Cloud credentials"),
        (".netrc".into(), "netrc credentials"),
        ("Library/Keychains".into(), "the macOS keychain"),
        (
            format!("Library/Application Support/{APP_IDENTIFIER}"),
            "the app's own data",
        ),
        (
            format!(".local/share/{APP_IDENTIFIER}"),
            "the app's own data",
        ),
        (format!(".config/{APP_IDENTIFIER}"), "the app's own data"),
        (
            format!("AppData/Roaming/{APP_IDENTIFIER}"),
            "the app's own data",
        ),
        (
            format!("AppData/Local/{APP_IDENTIFIER}"),
            "the app's own data",
        ),
    ]
}

// Absolute system locations. The /etc and /var forms are listed beside their
// /private targets because macOS reaches them through symlinks.
fn system_denied() -> Vec<(String, &'static str)> {
    let mut list: Vec<(String, &'static str)> = [
        "/etc",
        "/private/etc",
        "/System",
        "/usr",
        "/bin",
        "/sbin",
        "/var/db",
        "/private/var/db",
        "/Library/Keychains",
    ]
    .iter()
    .map(|p| ((*p).to_string(), "a system folder"))
    .collect();
    if let Some(root) = std::env::var_os("SYSTEMROOT").filter(|v| !v.is_empty()) {
        list.push((
            PathBuf::from(root).to_string_lossy().into_owned(),
            "the Windows folder",
        ));
    }
    list
}

// The folders a `..` may lead back into: the home directory and the temp directory.
#[derive(Debug, Clone, Default)]
pub struct Roots {
    home: Option<PathBuf>,
    temp: Option<PathBuf>,
}

impl Roots {
    pub fn current() -> Roots {
        let home = std::env::var_os("HOME")
            .or_else(|| std::env::var_os("USERPROFILE"))
            .map(PathBuf::from);
        Roots {
            home: home.and_then(|h| std::fs::canonicalize(h).ok()),
            temp: std::fs::canonicalize(std::env::temp_dir()).ok(),
        }
    }

    fn allows(&self, canonical: &Path) -> bool {
        [self.home.as_deref(), self.temp.as_deref()]
            .into_iter()
            .flatten()
            .any(|root| starts_with(canonical, root))
    }
}

// ---------------------------------------------------------------- file paths

// Reads a file for a Read file node. Returns the canonical path to read.
pub fn read_path(raw: &str) -> AppResult<PathBuf> {
    vet_read(raw, &Roots::current())
}

// Chooses a file for a Save file node. Returns the canonical path to write.
pub fn save_path(raw: &str) -> AppResult<PathBuf> {
    vet_save(raw, &Roots::current())
}

fn vet_read(raw: &str, roots: &Roots) -> AppResult<PathBuf> {
    let requested = absolute(raw)?;
    let canonical = std::fs::canonicalize(&requested)
        .map_err(|_| AppError::invalid_input(format!("Could not find {raw}.")))?;
    check(raw, &canonical, roots)?;
    Ok(canonical)
}

fn vet_save(raw: &str, roots: &Roots) -> AppResult<PathBuf> {
    let requested = absolute(raw)?;
    // An existing entry is resolved in full, so a link to ~/.ssh is caught. A missing
    // one is placed under its canonical folder, so `..` in the folders is resolved too.
    let target = if std::fs::symlink_metadata(&requested).is_ok() {
        std::fs::canonicalize(&requested).map_err(|_| {
            AppError::invalid_input(format!("{raw} is a link to a file that does not exist."))
        })?
    } else {
        let name = requested
            .file_name()
            .ok_or_else(|| AppError::invalid_input(format!("{raw} does not name a file.")))?;
        let parent = requested
            .parent()
            .ok_or_else(|| AppError::invalid_input(format!("{raw} does not name a file.")))?;
        let parent = std::fs::canonicalize(parent).map_err(|_| {
            AppError::invalid_input(format!("The folder for {raw} does not exist."))
        })?;
        parent.join(name)
    };
    check(raw, &target, roots)?;
    Ok(target)
}

fn absolute(raw: &str) -> AppResult<PathBuf> {
    let path = Path::new(raw);
    if path.is_absolute() {
        Ok(path.to_path_buf())
    } else {
        Err(AppError::invalid_input(format!("\"{raw}\" is not an absolute path. Choose the file with Browse, or give its full path.")))
    }
}

fn check(raw: &str, canonical: &Path, roots: &Roots) -> AppResult<()> {
    let climbs = Path::new(raw)
        .components()
        .any(|c| matches!(c, Component::ParentDir));
    if climbs && !roots.allows(canonical) {
        return Err(AppError::invalid_input(format!("{raw} climbs with \"..\" out of your home and temp folders. Use the full path instead.")));
    }
    if let Some(what) = sensitive_location(canonical, roots.home.as_deref()) {
        return Err(AppError::invalid_input(format!("Blocked: {raw} is in a protected location ({what}). Workflows cannot read or write there.")));
    }
    Ok(())
}

// The reason a canonical path is protected, or None. Pure, so the rules are testable.
fn sensitive_location(canonical: &Path, home: Option<&Path>) -> Option<&'static str> {
    let actual = parts(canonical);
    if actual.iter().any(|p| p == "keychains") {
        return Some("a keychain");
    }
    if let Some(name) = canonical
        .file_name()
        .map(|n| n.to_string_lossy().to_lowercase())
    {
        if APP_OWN_FILE_PREFIXES
            .iter()
            .any(|prefix| name.starts_with(prefix))
        {
            return Some("the app's own data");
        }
    }
    if let Some(home) = home {
        let home_parts = parts(home);
        for (relative, what) in home_denied() {
            let mut prefix = home_parts.clone();
            prefix.extend(parts(Path::new(&relative)));
            if actual.starts_with(&prefix) {
                return Some(what);
            }
        }
    }
    for (absolute_path, what) in system_denied() {
        if actual.starts_with(&parts(Path::new(&absolute_path))) {
            return Some(what);
        }
    }
    None
}

// Lower-cased components, so `~/.SSH` matches `~/.ssh` on case-insensitive macOS
// volumes too. Being stricter on case-sensitive systems is the safe direction.
fn parts(path: &Path) -> Vec<String> {
    path.components()
        .filter_map(|c| match c {
            Component::CurDir => None,
            other => Some(other.as_os_str().to_string_lossy().to_lowercase()),
        })
        .collect()
}

fn starts_with(path: &Path, prefix: &Path) -> bool {
    parts(path).starts_with(&parts(prefix))
}

// ---------------------------------------------------------------- HTTP targets

// Parses a URL and keeps only http and https.
pub fn parse_http_url(raw: &str) -> AppResult<url::Url> {
    let url = url::Url::parse(raw).map_err(|_| AppError::invalid_input("The URL is not valid."))?;
    match url.scheme() {
        "http" | "https" => Ok(url),
        other => Err(AppError::invalid_input(format!(
            "Only http and https URLs can be called, not \"{other}\"."
        ))),
    }
}

// Vets an HTTP node's URL and returns the addresses it may connect to. Loopback,
// link-local, private and similar addresses are refused unless `allow_private` is
// set, and a hostname is refused when any address it resolves to is one of them.
pub async fn vet_http_target(raw: &str, allow_private: bool) -> AppResult<PinnedTarget> {
    let url = parse_http_url(raw)?;
    let port = url.port_or_known_default().unwrap_or(443);
    let blocked = |host: &str, reason: &str| {
        AppError::invalid_input(format!("Blocked: {host} is {reason}. Turn on \"Allow private network addresses\" in the HTTP node's settings to call it."))
    };
    match url.host() {
        None => Err(AppError::invalid_input("The URL has no host.")),
        Some(url::Host::Ipv4(ip)) => vet_literal(IpAddr::V4(ip), port, allow_private, &blocked),
        Some(url::Host::Ipv6(ip)) => vet_literal(IpAddr::V6(ip), port, allow_private, &blocked),
        Some(url::Host::Domain(name)) => {
            if !allow_private {
                if let Some(reason) = denied_host_name(name) {
                    return Err(blocked(name, reason));
                }
            }
            let addrs: Vec<SocketAddr> = tokio::net::lookup_host((name, port))
                .await
                .map_err(|_| AppError::invalid_input(format!("Could not resolve {name}.")))?
                .collect();
            if addrs.is_empty() {
                return Err(AppError::invalid_input(format!(
                    "Could not resolve {name}."
                )));
            }
            if !allow_private {
                if let Some(reason) = addrs.iter().find_map(|a| private_address_reason(a.ip())) {
                    return Err(blocked(name, &format!("resolving to {reason}")));
                }
            }
            Ok(PinnedTarget {
                host: Some(name.to_string()),
                addrs,
            })
        }
    }
}

fn vet_literal(
    ip: IpAddr,
    port: u16,
    allow_private: bool,
    blocked: &dyn Fn(&str, &str) -> AppError,
) -> AppResult<PinnedTarget> {
    if !allow_private {
        if let Some(reason) = private_address_reason(ip) {
            return Err(blocked(&ip.to_string(), reason));
        }
    }
    Ok(PinnedTarget {
        host: None,
        addrs: vec![SocketAddr::new(ip, port)],
    })
}

// Host names that point at the local machine or at cloud metadata, refused before any lookup.
fn denied_host_name(host: &str) -> Option<&'static str> {
    let host = host.trim_end_matches('.').to_lowercase();
    if host == "localhost" || host.ends_with(".localhost") {
        Some("a localhost name")
    } else if host == "metadata.google.internal" || host == "metadata" {
        Some("a cloud metadata name")
    } else {
        None
    }
}

// Why an address is not public, or None. Pure, so the ranges are testable.
pub fn private_address_reason(ip: IpAddr) -> Option<&'static str> {
    match ip {
        IpAddr::V4(v4) => v4_reason(v4),
        IpAddr::V6(v6) => match v6.to_ipv4_mapped() {
            Some(v4) => v4_reason(v4),
            None => v6_reason(v6),
        },
    }
}

fn v4_reason(ip: Ipv4Addr) -> Option<&'static str> {
    let o = ip.octets();
    if ip.is_loopback() {
        Some("a loopback address")
    } else if ip.is_link_local() {
        Some("a link-local address (cloud metadata lives here)")
    } else if ip.is_private() {
        Some("a private-network address")
    } else if o[0] == 0 {
        Some("a this-network address")
    } else if o[0] == 100 && (o[1] & 0xc0) == 64 {
        Some("a carrier-grade NAT address")
    } else if ip.is_unspecified() {
        Some("an unspecified address")
    } else if ip.is_multicast() || ip.is_broadcast() {
        Some("a multicast or broadcast address")
    } else {
        None
    }
}

fn v6_reason(ip: Ipv6Addr) -> Option<&'static str> {
    let first = ip.segments()[0];
    if ip.is_loopback() {
        Some("a loopback address")
    } else if ip.is_unspecified() {
        Some("an unspecified address")
    } else if first & 0xffc0 == 0xfe80 {
        Some("a link-local address (cloud metadata lives here)")
    } else if first & 0xfe00 == 0xfc00 {
        Some("a private-network address")
    } else if ip.is_multicast() {
        Some("a multicast address")
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // A throwaway folder layout: base/{home,temp,outside}. `home` plays the user's
    // home directory and `temp` the temp folder, so the rules run without touching
    // the real ones.
    struct Layout {
        base: PathBuf,
        roots: Roots,
    }

    impl Layout {
        fn new() -> Layout {
            let base =
                std::env::temp_dir().join(format!("db-free-policy-{}", uuid::Uuid::new_v4()));
            for dir in ["home/.ssh", "temp", "outside"] {
                std::fs::create_dir_all(base.join(dir)).unwrap();
            }
            std::fs::write(base.join("home/.ssh/id_rsa"), "secret").unwrap();
            std::fs::write(base.join("outside/secret.txt"), "outside").unwrap();
            std::fs::write(base.join("temp/data.csv"), "a,b\n1,2\n").unwrap();
            let roots = Roots {
                home: std::fs::canonicalize(base.join("home")).ok(),
                temp: std::fs::canonicalize(base.join("temp")).ok(),
            };
            Layout {
                base: std::fs::canonicalize(&base).unwrap(),
                roots,
            }
        }

        fn at(&self, rel: &str) -> String {
            self.base.join(rel).to_string_lossy().into_owned()
        }
    }

    impl Drop for Layout {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.base);
        }
    }

    #[test]
    fn a_relative_path_is_refused() {
        let layout = Layout::new();
        let err = vet_read("data.csv", &layout.roots).unwrap_err();
        assert!(err.to_string().contains("not an absolute path"), "{err}");
    }

    #[test]
    fn an_ordinary_file_in_the_temp_folder_is_read_through_its_canonical_path() {
        let layout = Layout::new();
        let path = vet_read(&layout.at("temp/data.csv"), &layout.roots).unwrap();
        assert_eq!(path, layout.base.join("temp/data.csv"));
    }

    #[test]
    fn a_missing_file_is_reported_as_not_found() {
        let layout = Layout::new();
        let err = vet_read(&layout.at("temp/nope.csv"), &layout.roots).unwrap_err();
        assert!(err.to_string().contains("Could not find"), "{err}");
    }

    #[test]
    fn dot_dot_that_stays_inside_an_allowed_folder_is_fine() {
        let layout = Layout::new();
        assert!(vet_read(&layout.at("temp/../temp/data.csv"), &layout.roots).is_ok());
    }

    #[test]
    fn dot_dot_that_climbs_out_of_the_allowed_folders_is_refused() {
        let layout = Layout::new();
        let err = vet_read(&layout.at("home/../outside/secret.txt"), &layout.roots).unwrap_err();
        assert!(err.to_string().contains("climbs with"), "{err}");
    }

    #[test]
    fn ssh_keys_are_refused_even_by_a_plain_path() {
        let layout = Layout::new();
        let err = vet_read(&layout.at("home/.ssh/id_rsa"), &layout.roots).unwrap_err();
        assert!(err.to_string().contains("SSH keys"), "{err}");
    }

    #[test]
    fn the_credential_folder_is_matched_whatever_its_case() {
        let home = Path::new("/Users/alice");
        assert_eq!(
            sensitive_location(Path::new("/Users/alice/.SSH/id_rsa"), Some(home)),
            Some("SSH keys")
        );
        assert_eq!(
            sensitive_location(Path::new("/Users/alice/.Aws/credentials"), Some(home)),
            Some("AWS credentials")
        );
    }

    #[test]
    fn a_symlink_into_a_protected_folder_is_refused() {
        let layout = Layout::new();
        let link = layout.base.join("temp/link.csv");
        std::os::unix::fs::symlink(layout.base.join("home/.ssh/id_rsa"), &link).unwrap();
        let err = vet_read(&link.to_string_lossy(), &layout.roots).unwrap_err();
        assert!(err.to_string().contains("SSH keys"), "{err}");
    }

    #[test]
    fn saving_a_new_file_in_the_temp_folder_is_allowed() {
        let layout = Layout::new();
        let target = vet_save(&layout.at("temp/out.json"), &layout.roots).unwrap();
        assert_eq!(target, layout.base.join("temp/out.json"));
    }

    #[test]
    fn saving_into_a_protected_folder_is_refused() {
        let layout = Layout::new();
        let err = vet_save(&layout.at("home/.ssh/authorized_keys"), &layout.roots).unwrap_err();
        assert!(err.to_string().contains("SSH keys"), "{err}");
    }

    #[test]
    fn saving_into_a_missing_folder_says_so() {
        let layout = Layout::new();
        let err = vet_save(&layout.at("temp/missing/out.csv"), &layout.roots).unwrap_err();
        assert!(err.to_string().contains("does not exist"), "{err}");
    }

    #[test]
    fn saving_through_a_dangling_link_is_refused() {
        let layout = Layout::new();
        let link = layout.base.join("temp/dangling.csv");
        std::os::unix::fs::symlink(layout.base.join("home/.ssh/new_key"), &link).unwrap();
        assert!(vet_save(&link.to_string_lossy(), &layout.roots).is_err());
    }

    #[test]
    fn the_app_database_and_master_key_are_refused_wherever_they_sit() {
        assert_eq!(
            sensitive_location(Path::new("/tmp/x/db-free.sqlite"), None),
            Some("the app's own data")
        );
        assert_eq!(
            sensitive_location(Path::new("/tmp/x/db-free.sqlite-wal"), None),
            Some("the app's own data")
        );
        assert_eq!(
            sensitive_location(Path::new("/tmp/x/master.key"), None),
            Some("the app's own data")
        );
        let home = Path::new("/Users/alice");
        assert_eq!(
            sensitive_location(
                Path::new("/Users/alice/Library/Application Support/app.dbfree.desktop/notes.txt"),
                Some(home)
            ),
            Some("the app's own data")
        );
        assert_eq!(
            sensitive_location(
                Path::new("/Users/alice/Library/Keychains/login.keychain-db"),
                Some(home)
            ),
            Some("a keychain")
        );
        assert_eq!(
            sensitive_location(Path::new("/Users/alice/Documents/report.csv"), Some(home)),
            None
        );
    }

    #[test]
    fn system_folders_are_refused_without_touching_the_disk() {
        for path in [
            "/etc/passwd",
            "/private/etc/hosts",
            "/System/Library/x",
            "/usr/bin/env",
            "/private/var/db/dslocal/x",
        ] {
            assert!(
                sensitive_location(Path::new(path), None).is_some(),
                "{path} should be protected"
            );
        }
        assert_eq!(
            sensitive_location(Path::new("/private/var/folders/ab/T/file.csv"), None),
            None
        );
        assert_eq!(
            sensitive_location(Path::new("/etcetera/file"), None),
            None,
            "a folder that only shares a prefix is not /etc"
        );
    }

    #[test]
    fn only_http_and_https_are_called() {
        assert!(parse_http_url("https://api.example.com/items").is_ok());
        assert!(parse_http_url("http://api.example.com/items").is_ok());
        for raw in [
            "file:///etc/passwd",
            "ftp://example.com/x",
            "gopher://example.com/",
        ] {
            let err = parse_http_url(raw).unwrap_err();
            assert!(
                err.to_string().contains("Only http and https"),
                "{raw}: {err}"
            );
        }
        assert!(parse_http_url("not a url").is_err());
    }

    #[test]
    fn private_and_special_ranges_are_recognised() {
        let blocked = [
            "127.0.0.1",
            "127.1.2.3",
            "::1",
            "169.254.169.254",
            "169.254.0.1",
            "fe80::1",
            "10.0.0.1",
            "172.16.0.1",
            "172.31.255.255",
            "192.168.1.1",
            "100.100.100.200",
            "0.0.0.0",
            "::",
            "fd00::1",
            "::ffff:127.0.0.1",
            "::ffff:169.254.169.254",
            "224.0.0.1",
        ];
        for ip in blocked {
            assert!(
                private_address_reason(ip.parse().unwrap()).is_some(),
                "{ip} should be refused"
            );
        }
        let public = [
            "8.8.8.8",
            "1.1.1.1",
            "172.32.0.1",
            "172.15.255.255",
            "100.128.0.1",
            "2606:4700:4700::1111",
            "::ffff:8.8.8.8",
        ];
        for ip in public {
            assert!(
                private_address_reason(ip.parse().unwrap()).is_none(),
                "{ip} should be allowed"
            );
        }
    }

    #[test]
    fn local_host_names_are_refused() {
        assert!(denied_host_name("localhost").is_some());
        assert!(denied_host_name("api.localhost").is_some());
        assert!(denied_host_name("metadata.google.internal").is_some());
        assert!(denied_host_name("api.example.com").is_none());
    }

    #[tokio::test]
    async fn literal_addresses_are_refused_unless_opted_in() {
        let err = vet_http_target("http://127.0.0.1:8080/x", false)
            .await
            .unwrap_err();
        assert!(err.to_string().contains("loopback"), "{err}");
        assert!(
            vet_http_target("https://169.254.169.254/latest/meta-data", false)
                .await
                .is_err()
        );
        assert!(vet_http_target("http://[::1]:9/", false).await.is_err());

        let pinned = vet_http_target("http://127.0.0.1:8080/x", true)
            .await
            .unwrap();
        assert_eq!(pinned.host, None);
        assert_eq!(
            pinned.addrs,
            vec![SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 8080)]
        );
    }

    #[tokio::test]
    async fn localhost_by_name_is_refused_before_any_lookup() {
        let err = vet_http_target("http://localhost:8080/", false)
            .await
            .unwrap_err();
        assert!(err.to_string().contains("localhost"), "{err}");
    }

    #[tokio::test]
    async fn a_public_literal_address_is_pinned_without_a_lookup() {
        let pinned = vet_http_target("https://8.8.8.8/dns", false).await.unwrap();
        assert_eq!(
            pinned.addrs,
            vec![SocketAddr::new(IpAddr::V4(Ipv4Addr::new(8, 8, 8, 8)), 443)]
        );
    }
}
