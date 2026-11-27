use std::io::Read;
use std::net::{IpAddr, ToSocketAddrs};
use std::time::{Duration, Instant};

use reqwest::blocking::{Client, Response};
use reqwest::header::{ACCEPT, CONTENT_TYPE, LOCATION};
use tauri::Url;

use crate::platform::parse_http_url;

const FETCH_TIMEOUT: Duration = Duration::from_secs(15);
const MAX_REDIRECTS: usize = 4;
const MAX_BYTES: u64 = 2 * 1024 * 1024;

pub(super) struct Article {
    pub title: String,
    pub body: String,
}

pub(super) fn fetch(url: &Url) -> Option<Article> {
    fetch_with(url, |url, remaining| {
        let host = url.host_str()?.trim_matches(['[', ']']);
        let hostname = host.trim_end_matches('.');
        if host.parse::<IpAddr>().is_err()
            && (!hostname.contains('.')
                || hostname.ends_with(".localhost")
                || hostname.ends_with(".local")
                || hostname.ends_with(".home.arpa"))
        {
            return None;
        }
        let addresses: Vec<_> = (host, url.port_or_known_default()?)
            .to_socket_addrs()
            .ok()?
            .collect();
        if addresses.is_empty()
            || addresses
                .iter()
                .any(|address| !public_address(address.ip()))
        {
            return None;
        }
        let client = Client::builder()
            .no_proxy()
            .resolve_to_addrs(host, &addresses)
            .redirect(reqwest::redirect::Policy::none())
            .connect_timeout(Duration::from_secs(5).min(remaining))
            .timeout(remaining)
            .user_agent("Wikra")
            .build()
            .ok()?;
        client
            .get(url.clone())
            .header(
                ACCEPT,
                "text/html, application/xhtml+xml, text/markdown, text/plain",
            )
            .send()
            .ok()
    })
}

fn fetch_with(
    start: &Url,
    mut request: impl FnMut(&Url, Duration) -> Option<Response>,
) -> Option<Article> {
    let deadline = Instant::now() + FETCH_TIMEOUT;
    let mut url = start.clone();
    for hop in 0..=MAX_REDIRECTS {
        url.set_fragment(None);
        let response = request(&url, deadline.checked_duration_since(Instant::now())?)?;
        let status = response.status();
        if status.is_redirection() && hop < MAX_REDIRECTS {
            let location = response.headers().get(LOCATION)?.to_str().ok()?;
            let next = url.join(location).ok()?;
            url = parse_http_url(next.as_str())?;
            continue;
        }
        return status
            .is_success()
            .then(|| read_response(response))
            .flatten();
    }
    None
}

fn public_address(address: IpAddr) -> bool {
    match address {
        IpAddr::V4(ip) => {
            let [a, b, c, _] = ip.octets();
            !matches!(a, 0 | 10 | 127 | 224..=255)
                && !matches!(
                    (a, b),
                    (100, 64..=127) | (169, 254) | (172, 16..=31) | (192, 168) | (198, 18..=19)
                )
                && !matches!(
                    (a, b, c),
                    (192, 0, 0 | 2) | (192, 88, 99) | (198, 51, 100) | (203, 0, 113)
                )
        }
        IpAddr::V6(ip) => {
            // Only public unicast. Exclude protocol assignments, transition and documentation ranges.
            let [a, b, ..] = ip.segments();
            (0x2000..=0x3fff).contains(&a)
                && !(a == 0x2001 && (b <= 0x01ff || b == 0x0db8))
                && a != 0x2002
                && a != 0x3fff
        }
    }
}

fn read_response(response: Response) -> Option<Article> {
    let media_type = response
        .headers()
        .get(CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .map(|value| {
            value
                .split(';')
                .next()
                .unwrap_or_default()
                .trim()
                .to_ascii_lowercase()
        });
    if media_type.as_deref().is_some_and(|kind| {
        !matches!(
            kind,
            "text/html" | "application/xhtml+xml" | "text/markdown" | "text/plain"
        )
    }) || response.content_length().is_some_and(|len| len > MAX_BYTES)
    {
        return None;
    }
    let mut bytes = Vec::new();
    response.take(MAX_BYTES + 1).read_to_end(&mut bytes).ok()?;
    if bytes.len() as u64 > MAX_BYTES {
        return None;
    }
    let text = std::str::from_utf8(&bytes)
        .ok()?
        .trim_matches('\u{feff}')
        .trim();
    if matches!(
        media_type.as_deref(),
        Some("text/html" | "application/xhtml+xml")
    ) || media_type.is_none() && text.starts_with('<')
    {
        return html_article(text);
    }
    (!text.is_empty() && !text.contains('\0')).then(|| Article {
        title: String::new(),
        body: text.to_owned(),
    })
}

fn html_article(html: &str) -> Option<Article> {
    let lowercase_html = html.to_ascii_lowercase();
    let mut buffers = [String::new(), String::new(), String::new()];
    let mut depth = [1usize, 0, 0];
    let mut hidden = Vec::new();
    let mut title = String::new();
    let mut in_title = false;
    let mut password_form = false;
    let mut rest = html;
    while !rest.is_empty() {
        let next = rest.find('<').unwrap_or(rest.len());
        let text = &rest[..next];
        rest = &rest[next..];
        if in_title {
            append_text(&mut title, text);
        } else if hidden.is_empty() {
            apply_visible(&mut buffers, depth, |body| append_text(body, text));
        }
        if rest.is_empty() {
            break;
        }
        if let Some(comment) = rest.strip_prefix("<!--") {
            rest = comment.split_once("-->").map_or("", |(_, tail)| tail);
            continue;
        }
        let Some(end) = tag_end(rest) else {
            rest = &rest[1..];
            if hidden.is_empty() {
                apply_visible(&mut buffers, depth, |body| body.push_str("&lt;"));
            }
            continue;
        };
        let tag = &rest[1..end];
        rest = &rest[end + 1..];
        let closing = tag.starts_with('/');
        let tag = tag.trim_start_matches('/');
        let split = tag
            .find(|c: char| !c.is_ascii_alphanumeric() && c != '-')
            .unwrap_or(tag.len());
        let name = tag[..split].to_ascii_lowercase();
        if name.is_empty() {
            continue;
        }
        if name == "title" && hidden.last().is_none_or(|name| name == "head") {
            in_title = !closing;
            continue;
        }
        if !closing
            && name == "input"
            && attribute(&tag[split..], "type")
                .is_some_and(|value| value.eq_ignore_ascii_case("password"))
        {
            password_form = true;
        }
        if !closing && matches!(name.as_str(), "script" | "style") {
            // Raw-text elements may contain '<' that is unrelated to markup.
            let close = format!("</{name}");
            let lowercase_rest = &lowercase_html[html.len() - rest.len()..];
            rest = lowercase_rest
                .find(&close)
                .and_then(|start| rest[start..].split_once('>'))
                .map_or("", |(_, tail)| tail);
            continue;
        }
        if matches!(
            name.as_str(),
            "head" | "template" | "noscript" | "svg" | "nav" | "footer" | "aside" | "form"
        ) {
            if closing {
                if hidden.last() == Some(&name) {
                    hidden.pop();
                }
            } else if !tag.ends_with('/') {
                hidden.push(name);
            }
            continue;
        }
        if !hidden.is_empty() {
            continue;
        }
        if let Some(index) = match name.as_str() {
            "main" => Some(1),
            "article" => Some(2),
            _ => None,
        } {
            apply_visible(&mut buffers, depth, paragraph_break);
            depth[index] = if closing {
                depth[index].saturating_sub(1)
            } else {
                depth[index] + 1
            };
        }
        if matches!(
            name.as_str(),
            "p" | "div"
                | "section"
                | "blockquote"
                | "li"
                | "ul"
                | "ol"
                | "pre"
                | "h1"
                | "h2"
                | "h3"
                | "h4"
                | "h5"
                | "h6"
        ) {
            apply_visible(&mut buffers, depth, |body| {
                paragraph_break(body);
                if !closing && name.starts_with('h') {
                    let level = usize::from(name.as_bytes()[1] - b'0');
                    body.push_str(&"#".repeat(level));
                    body.push(' ');
                }
            });
        } else if name == "br" {
            apply_visible(&mut buffers, depth, paragraph_break);
        }
    }
    let body = buffers
        .iter()
        .rev()
        .map(|s| s.trim())
        .find(|s| !s.is_empty())?;
    let title = title.trim();
    let lowercase_title = title.to_lowercase();
    let wall = [
        "sign in",
        "log in",
        "access denied",
        "just a moment",
        "登录",
        "访问受限",
    ]
    .iter()
    .any(|marker| lowercase_title.starts_with(marker));
    let paywall_markers = [
        "subscribe to continue",
        "sign in to continue",
        "log in to continue",
        "登录后继续阅读",
        "登录后查看全文",
    ];
    let paywall = body
        .split("\n\n")
        .map(str::trim)
        .filter(|block| !block.starts_with('#'))
        .take(3)
        .map(str::to_lowercase)
        .any(|block| {
            paywall_markers
                .iter()
                .any(|marker| block.starts_with(marker))
        });
    if wall
        || paywall
        || password_form && body.chars().take(160).count() < 160
        || !body.chars().any(char::is_alphanumeric)
    {
        return None;
    }
    Some(Article {
        title: title.to_owned(),
        body: body.to_owned(),
    })
}

fn apply_visible(buffers: &mut [String; 3], depth: [usize; 3], mut f: impl FnMut(&mut String)) {
    for (buffer, level) in buffers.iter_mut().zip(depth) {
        if level > 0 {
            f(buffer);
        }
    }
}

fn paragraph_break(body: &mut String) {
    body.truncate(body.trim_end().len());
    if !body.is_empty() {
        body.push_str("\n\n");
    }
}

fn tag_end(rest: &str) -> Option<usize> {
    let first = *rest.as_bytes().get(1)?;
    if !first.is_ascii_alphabetic() && !matches!(first, b'/' | b'!') {
        return None;
    }
    let mut quote = None;
    for (index, ch) in rest.char_indices().skip(1).take(4096) {
        if let Some(open) = quote {
            if ch == open {
                quote = None;
            }
        } else if matches!(ch, '"' | '\'') {
            quote = Some(ch);
        } else if ch == '>' {
            return Some(index);
        } else if ch == '<' {
            break;
        }
    }
    None
}

fn attribute<'a>(mut attrs: &'a str, wanted: &str) -> Option<&'a str> {
    while !attrs.trim_start().is_empty() {
        attrs = attrs.trim_start();
        let len = attrs
            .find(|c: char| c.is_whitespace() || c == '=')
            .unwrap_or(attrs.len());
        let (name, rest) = attrs.split_at(len);
        attrs = rest.trim_start();
        let Some(value) = attrs.strip_prefix('=') else {
            continue;
        };
        let value = value.trim_start();
        let (value, remaining) = if let Some(quote @ ('"' | '\'')) = value.chars().next() {
            value[1..].split_once(quote)?
        } else {
            value.split_at(value.find(char::is_whitespace).unwrap_or(value.len()))
        };
        if name.eq_ignore_ascii_case(wanted) {
            return Some(value);
        }
        attrs = remaining;
    }
    None
}

fn append_text(output: &mut String, mut input: &str) {
    while let Some(ch) = input.chars().next() {
        input = &input[ch.len_utf8()..];
        let ch = if ch == '&'
            && let Some(end) = input
                .as_bytes()
                .iter()
                .take(13)
                .position(|&byte| byte == b';')
            && let Some(decoded) = decode_entity(&input[..end])
        {
            input = &input[end + 1..];
            decoded
        } else {
            ch
        };
        match ch {
            '<' => output.push_str("&lt;"),
            '>' => output.push_str("&gt;"),
            '[' => output.push_str("&#91;"),
            ']' => output.push_str("&#93;"),
            ch if ch.is_whitespace() => {
                if !output.is_empty() && !output.ends_with([' ', '\n']) {
                    output.push(' ');
                }
            }
            ch if !ch.is_control() => output.push(ch),
            _ => (),
        }
    }
}

fn decode_entity(entity: &str) -> Option<char> {
    let numeric = |value, radix| {
        u32::from_str_radix(value, radix)
            .ok()
            .and_then(char::from_u32)
    };
    match entity {
        "amp" => Some('&'),
        "lt" => Some('<'),
        "gt" => Some('>'),
        "nbsp" => Some(' '),
        "quot" => Some('"'),
        "apos" => Some('\''),
        "ndash" => Some('–'),
        "mdash" => Some('—'),
        "hellip" => Some('…'),
        _ => {
            if let Some(value) = entity
                .strip_prefix("#x")
                .or_else(|| entity.strip_prefix("#X"))
            {
                numeric(value, 16)
            } else {
                numeric(entity.strip_prefix('#')?, 10)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::io::Write;
    use std::net::TcpListener;

    use super::*;

    #[test]
    fn reqwest_extracts_article_and_degrades_http_login_and_empty_bodies() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let server = std::thread::spawn(move || {
            for (status, body) in [
                (
                    "200 OK",
                    "<title>阅读 &amp; 记录</title><nav>导航</nav><main><h1>标题</h1><p>第一段。</p><script>let x = '<b>';</script><p>第二段 &#x4e2d;。</p></main>",
                ),
                ("401 Unauthorized", "restricted"),
                ("403 Forbidden", "restricted"),
                (
                    "200 OK",
                    "<html><title>Sign in</title><form><input type='password'></form></html>",
                ),
                ("200 OK", "<script>only_javascript()</script>"),
            ] {
                let (mut socket, _) = listener.accept().unwrap();
                socket
                    .set_read_timeout(Some(Duration::from_secs(2)))
                    .unwrap();
                let mut request = [0; 4096];
                let size = socket.read(&mut request).unwrap();
                let request = String::from_utf8_lossy(&request[..size]).to_ascii_lowercase();
                assert!(!request.contains("cookie:") && !request.contains("authorization:"));
                write!(socket, "HTTP/1.1 {status}\r\nContent-Type: text/html\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).unwrap();
            }
        });
        let url = Url::parse(&format!("http://{address}/article")).unwrap();
        assert!(
            fetch(&url).is_none(),
            "production fetch blocks loopback before connecting"
        );
        let client = Client::builder()
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(Duration::from_secs(2))
            .build()
            .unwrap();
        let get = || fetch_with(&url, |url, _| client.get(url.clone()).send().ok());
        let article = get().unwrap();
        assert_eq!(article.title, "阅读 & 记录");
        assert_eq!(article.body, "# 标题\n\n第一段。\n\n第二段 中。");
        for _ in 0..4 {
            assert!(get().is_none());
        }
        server.join().unwrap();
    }

    #[test]
    fn denies_nonpublic_addresses_and_accepts_public_dns_results() {
        for address in [
            "127.0.0.1",
            "10.0.0.1",
            "100.64.0.1",
            "169.254.169.254",
            "172.16.0.2",
            "192.168.1.1",
            "0.0.0.0",
            "::1",
            "fc00::1",
            "::ffff:172.16.0.1",
            "fe80::1",
        ] {
            assert!(!public_address(address.parse().unwrap()), "{address}");
        }
        assert!(public_address("8.8.8.8".parse().unwrap()));
        assert!(public_address("2606:4700:4700::1111".parse().unwrap()));
    }
}
