/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */

//! Turns whatever the user typed into the address bar into a URL to load.

use url::Url;

/// The search engine used when the address bar input is not a URL.
/// `%s` is replaced with the percent-encoded query.
pub const DEFAULT_SEARCH_URL: &str = "https://duckduckgo.com/?q=%s";

/// Schemes that are loaded as typed.
const KNOWN_SCHEMES: &[&str] = &[
    "http",
    "https",
    "file",
    "about",
    "data",
    "view-source",
    "servo",
    "berick",
];

/// Resolve address bar input into a URL. Returns `None` for empty input.
///
/// - Full URLs with a known scheme load as typed (`https://example.com`).
/// - Things that look like a host get a scheme added (`example.com`,
///   `localhost:8080`, `192.168.1.1`). Local hosts get `http://`, everything else `https://`.
/// - Anything else becomes a search.
pub fn resolve(input: &str, search_url: &str) -> Option<Url> {
    let input = input.trim();
    if input.is_empty() {
        return None;
    }

    if let Ok(url) = Url::parse(input) {
        if KNOWN_SCHEMES.contains(&url.scheme()) {
            return Some(url);
        }
    }

    if looks_like_host(input) {
        let scheme = if is_local_host(input) {
            "http"
        } else {
            "https"
        };
        if let Ok(url) = Url::parse(&format!("{scheme}://{input}")) {
            return Some(url);
        }
    }

    search(input, search_url)
}

fn search(query: &str, search_url: &str) -> Option<Url> {
    let encoded: String = url::form_urlencoded::byte_serialize(query.as_bytes()).collect();
    Url::parse(&search_url.replace("%s", &encoded)).ok()
}

/// The host part of the input, without any port, path, query or fragment.
fn host_part(input: &str) -> &str {
    let end = input.find(['/', '?', '#']).unwrap_or(input.len());
    let host_and_port = &input[..end];
    // Bracketed IPv6 literal, like [::1]:8080.
    if host_and_port.starts_with('[') {
        return host_and_port
            .find(']')
            .map_or(host_and_port, |i| &host_and_port[..=i]);
    }
    host_and_port
        .rsplit_once(':')
        .filter(|(_, port)| !port.is_empty() && port.chars().all(|c| c.is_ascii_digit()))
        .map_or(host_and_port, |(host, _)| host)
}

fn looks_like_host(input: &str) -> bool {
    if input.contains(char::is_whitespace) {
        return false;
    }
    let host = host_part(input);
    if host.is_empty() {
        return false;
    }
    if host.eq_ignore_ascii_case("localhost") || host.starts_with('[') {
        return true;
    }
    // Needs a dot, with something on both sides of it, and a TLD that isn't all digits
    // unless the whole thing is an IPv4 address.
    let Some((rest, tld)) = host.rsplit_once('.') else {
        return false;
    };
    if rest.is_empty() || tld.is_empty() {
        return false;
    }
    let is_ipv4 = host.parse::<std::net::Ipv4Addr>().is_ok();
    is_ipv4
        || (!tld.chars().all(|c| c.is_ascii_digit())
            && host
                .chars()
                .all(|c| c.is_alphanumeric() || c == '-' || c == '.'))
}

fn is_local_host(input: &str) -> bool {
    let host = host_part(input)
        .trim_start_matches('[')
        .trim_end_matches(']');
    if host.eq_ignore_ascii_case("localhost") {
        return true;
    }
    match host.parse::<std::net::IpAddr>() {
        Ok(std::net::IpAddr::V4(ip)) => ip.is_loopback() || ip.is_private() || ip.is_link_local(),
        Ok(std::net::IpAddr::V6(ip)) => ip.is_loopback(),
        Err(_) => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn r(input: &str) -> String {
        resolve(input, DEFAULT_SEARCH_URL).unwrap().to_string()
    }

    #[test]
    fn empty_is_none() {
        assert!(resolve("   ", DEFAULT_SEARCH_URL).is_none());
    }

    #[test]
    fn full_urls_load_as_typed() {
        assert_eq!(r("https://servo.org/"), "https://servo.org/");
        assert_eq!(r("http://example.com/a?b=c"), "http://example.com/a?b=c");
        assert_eq!(r("about:blank"), "about:blank");
        assert_eq!(r("file:///tmp/x.html"), "file:///tmp/x.html");
    }

    #[test]
    fn bare_hosts_get_https() {
        assert_eq!(r("example.com"), "https://example.com/");
        assert_eq!(r("servo.org/blog"), "https://servo.org/blog");
        assert_eq!(r("  github.com  "), "https://github.com/");
        assert_eq!(
            r("sub.domain.co.uk/path?q=1"),
            "https://sub.domain.co.uk/path?q=1"
        );
    }

    #[test]
    fn local_hosts_get_http() {
        assert_eq!(r("localhost"), "http://localhost/");
        assert_eq!(r("localhost:8080"), "http://localhost:8080/");
        assert_eq!(r("127.0.0.1:3000/x"), "http://127.0.0.1:3000/x");
        assert_eq!(r("192.168.1.1"), "http://192.168.1.1/");
        assert_eq!(r("[::1]:8000"), "http://[::1]:8000/");
    }

    #[test]
    fn public_ip_gets_https() {
        assert_eq!(r("1.1.1.1"), "https://1.1.1.1/");
    }

    #[test]
    fn everything_else_is_a_search() {
        assert_eq!(r("rust servo"), "https://duckduckgo.com/?q=rust+servo");
        assert_eq!(r("servo"), "https://duckduckgo.com/?q=servo");
        assert_eq!(r("what is 1.5"), "https://duckduckgo.com/?q=what+is+1.5");
        assert_eq!(r("c++"), "https://duckduckgo.com/?q=c%2B%2B");
        assert_eq!(r("3.14"), "https://duckduckgo.com/?q=3.14");
        assert_eq!(r("foo.bar baz"), "https://duckduckgo.com/?q=foo.bar+baz");
    }

    #[test]
    fn custom_search_engine() {
        assert_eq!(
            resolve("hello", "https://www.google.com/search?q=%s")
                .unwrap()
                .as_str(),
            "https://www.google.com/search?q=hello"
        );
    }
}
