use std::fs;
use std::time::Duration;

use anyhow::{Context, bail};
use chrono::{DateTime, Local, TimeZone};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewsItem {
    pub title: String,
    pub link: String,
    /// Unix, a partir de `<pubDate>`.
    pub published: i64,
    /// `YYYY-MM-DD` en hora local.
    pub date: String,
    pub body: String,
}

pub const NEWS_URL: &str = "https://archlinux.org/feeds/news/";
const TIMEOUT: Duration = Duration::from_secs(10);
const PACMAN_LOG: &str = "/var/log/pacman.log";
const UPGRADE_MARKER: &str = "[PACMAN] starting full system upgrade";
const LOG_TIME_FORMAT: &str = "%Y-%m-%dT%H:%M:%S%z";
const CDATA_OPEN: &str = "<![CDATA[";
const CDATA_CLOSE: &str = "]]>";
/// Longitud que se mira como máximo para una entidad: `&#x10FFFF;` ocupa 10 caracteres.
const MAX_ENTITY_CHARS: usize = 12;

pub fn fetch() -> anyhow::Result<Vec<NewsItem>> {
    let agent = ureq::AgentBuilder::new()
        .timeout(TIMEOUT)
        .user_agent(concat!("facpm/", env!("CARGO_PKG_VERSION")))
        .build();
    let response = match agent.get(NEWS_URL).call() {
        Ok(response) => response,
        Err(ureq::Error::Status(code, _)) => bail!("el feed de noticias respondió {code}"),
        Err(err) => bail!("no se pudo descargar el feed de noticias: {err}"),
    };
    if response.status() != 200 {
        bail!("el feed de noticias respondió {}", response.status());
    }
    let xml = response
        .into_string()
        .context("no se pudo leer el feed de noticias")?;
    let items = parse_rss(&xml);
    if items.is_empty() {
        bail!("el feed de noticias no tiene entradas");
    }
    Ok(items)
}

/// Los `<item>` en orden. Los que no tienen `<pubDate>` RFC 2822 válida se descartan.
pub fn parse_rss(xml: &str) -> Vec<NewsItem> {
    let mut items = Vec::new();
    let mut rest = xml;
    while let Some(body_start) = open_tag_end(rest, "item") {
        let Some(len) = rest[body_start..].find("</item>") else {
            break;
        };
        items.extend(parse_item(&rest[body_start..body_start + len]));
        rest = &rest[body_start + len + "</item>".len()..];
    }
    items
}

fn parse_item(block: &str) -> Option<NewsItem> {
    let published = DateTime::parse_from_rfc2822(&field_text(element(block, "pubDate")?))
        .ok()?
        .timestamp();
    let text = |name: &str| field_text(element(block, name).unwrap_or(""));
    Some(NewsItem {
        title: text("title"),
        link: text("link"),
        published,
        date: local_date(published)?,
        body: html_to_text(&text("description")),
    })
}

fn local_date(timestamp: i64) -> Option<String> {
    let local = Local.timestamp_opt(timestamp, 0).single()?;
    Some(local.format("%Y-%m-%d").to_string())
}

fn open_tag_end(text: &str, name: &str) -> Option<usize> {
    let pattern = format!("<{name}");
    let mut from = 0;
    while let Some(pos) = text[from..].find(pattern.as_str()) {
        let after_name = from + pos + pattern.len();
        match text[after_name..].chars().next() {
            Some('>') => return Some(after_name + 1),
            Some(c) if c.is_whitespace() || c == '/' => {
                let gt = text[after_name..].find('>')?;
                return Some(after_name + gt + 1);
            }
            _ => from = after_name,
        }
    }
    None
}

fn element<'a>(block: &'a str, name: &str) -> Option<&'a str> {
    let start = open_tag_end(block, name)?;
    let len = block[start..].find(format!("</{name}>").as_str())?;
    Some(&block[start..start + len])
}

/// El texto escapado se decodifica una vez; el de `<![CDATA[...]]>` se toma literal.
fn field_text(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    let mut rest = raw;
    while let Some(start) = rest.find(CDATA_OPEN) {
        out.push_str(&decode_entities(&rest[..start]));
        let inner = &rest[start + CDATA_OPEN.len()..];
        let Some(end) = inner.find(CDATA_CLOSE) else {
            out.push_str(inner);
            return out.trim().to_string();
        };
        out.push_str(&inner[..end]);
        rest = &inner[end + CDATA_CLOSE.len()..];
    }
    out.push_str(&decode_entities(rest));
    out.trim().to_string()
}

/// HTML a texto plano: `</p>`, `<br>`, `</li>`, `</h1>`..`</h6>` y `</pre>` cortan la línea, y
/// `<li>` abre con "• ". Dentro de `<pre>` no se colapsan espacios. Lo demás se borra.
pub fn html_to_text(html: &str) -> String {
    let mut out = String::with_capacity(html.len());
    let mut rest = html;
    let mut in_pre = false;
    while let Some(lt) = rest.find('<') {
        push_text(&mut out, &rest[..lt], in_pre);
        let tail = &rest[lt..];
        let Some(gt) = tag_end(tail) else {
            // Un `<` que no abre etiqueta, como en "a < b", es texto.
            out.push('<');
            rest = &tail[1..];
            continue;
        };
        let (closing, name) = tag_name(&tail[1..gt]);
        match (closing, name.as_str()) {
            (false, "pre") => {
                ensure_line_break(&mut out);
                in_pre = true;
            }
            (true, "pre") => {
                in_pre = false;
                out.push('\n');
            }
            (false, "ul" | "ol") => ensure_line_break(&mut out),
            _ => out.push_str(tag_break(closing, &name)),
        }
        rest = &tail[gt + 1..];
    }
    push_text(&mut out, rest, in_pre);
    tidy(&out)
}

fn ensure_line_break(out: &mut String) {
    let line = out.trim_end_matches(' ');
    if !line.is_empty() && !line.ends_with('\n') {
        out.push('\n');
    }
}

fn tag_end(tail: &str) -> Option<usize> {
    let first = tail[1..].chars().next()?;
    if first.is_ascii_alphabetic() || first == '/' || first == '!' {
        tail.find('>')
    } else {
        None
    }
}

fn tag_name(tag: &str) -> (bool, String) {
    let (closing, name) = match tag.strip_prefix('/') {
        Some(name) => (true, name),
        None => (false, tag),
    };
    let name = name
        .chars()
        .take_while(|c| c.is_ascii_alphanumeric())
        .collect::<String>()
        .to_ascii_lowercase();
    (closing, name)
}

fn tag_break(closing: bool, name: &str) -> &'static str {
    match (closing, name) {
        (false, "br") => "\n",
        (false, "li") => "• ",
        (true, "p" | "li" | "h1" | "h2" | "h3" | "h4" | "h5" | "h6") => "\n",
        _ => "",
    }
}

fn push_text(out: &mut String, text: &str, in_pre: bool) {
    if in_pre {
        out.push_str(&decode_entities(text));
    } else {
        out.push_str(&decode_entities(&collapse_whitespace(text)));
    }
}

fn collapse_whitespace(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut in_space = false;
    for c in text.chars() {
        if !c.is_whitespace() {
            out.push(c);
            in_space = false;
        } else if !in_space {
            out.push(' ');
            in_space = true;
        }
    }
    out
}

fn tidy(text: &str) -> String {
    let mut lines: Vec<&str> = Vec::new();
    for line in text.lines().map(str::trim) {
        if line.is_empty() && lines.last().is_none_or(|last| last.is_empty()) {
            continue;
        }
        lines.push(line);
    }
    lines.join("\n").trim().to_string()
}

/// Decodifica una sola vez: `&amp;lt;` queda en `&lt;`.
fn decode_entities(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(amp) = rest.find('&') {
        out.push_str(&rest[..amp]);
        let tail = &rest[amp..];
        match decode_entity(tail) {
            Some((ch, len)) => {
                out.push(ch);
                rest = &tail[len..];
            }
            None => {
                out.push('&');
                rest = &tail[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

fn decode_entity(tail: &str) -> Option<(char, usize)> {
    let semi = tail
        .char_indices()
        .take(MAX_ENTITY_CHARS)
        .find(|&(_, c)| c == ';')?
        .0;
    let name = &tail[1..semi];
    let ch = match name {
        "amp" => '&',
        "lt" => '<',
        "gt" => '>',
        "quot" => '"',
        "apos" => '\'',
        _ => char::from_u32(char_code(name.strip_prefix('#')?)?)?,
    };
    Some((ch, semi + 1))
}

fn char_code(code: &str) -> Option<u32> {
    let (digits, radix) = match code.strip_prefix(['x', 'X']) {
        Some(hex) => (hex, 16),
        None => (code, 10),
    };
    if digits.is_empty() || !digits.chars().all(|c| c.is_digit(radix)) {
        return None;
    }
    u32::from_str_radix(digits, radix)
        .ok()
        .filter(|&value| value != 0)
}

/// Fecha del último `starting full system upgrade` del log de pacman.
pub fn last_upgrade() -> Option<i64> {
    let bytes = fs::read(PACMAN_LOG).ok()?;
    last_upgrade_from(&String::from_utf8_lossy(&bytes))
}

/// Versión de `last_upgrade` sobre un texto. Las líneas cuyo timestamp no se entiende se ignoran.
pub fn last_upgrade_from(log: &str) -> Option<i64> {
    log.lines()
        .filter(|line| line.contains(UPGRADE_MARKER))
        .filter_map(log_timestamp)
        .next_back()
}

fn log_timestamp(line: &str) -> Option<i64> {
    line.split('[').find_map(|piece| {
        let (inside, _) = piece.split_once(']')?;
        let parsed = DateTime::parse_from_str(inside, LOG_TIME_FORMAT).ok()?;
        Some(parsed.timestamp())
    })
}

/// ¿El título menciona a `name`? Compara token a token y sin mayúsculas: sirve `name` solo, o
/// `name-<dígito>...` (ej. `openssh-9.8p1`). Con un solo carácter cualquier palabra coincidiría.
pub fn mentions_package(title: &str, name: &str) -> bool {
    if name.chars().count() < 2 {
        return false;
    }
    tokens(&without_arch_linux(title)).any(|token| is_package_token(token, name))
}

/// `to_ascii_lowercase` conserva los bytes: las posiciones sirven para `title`.
fn without_arch_linux(title: &str) -> String {
    let lower = title.to_ascii_lowercase();
    let mut out = String::with_capacity(title.len());
    let mut last = 0;
    for (start, phrase) in lower.match_indices("arch linux") {
        out.push_str(&title[last..start]);
        last = start + phrase.len();
    }
    out.push_str(&title[last..]);
    out
}

fn tokens(text: &str) -> impl Iterator<Item = &str> {
    text.split(|c: char| {
        c.is_whitespace()
            || matches!(
                c,
                ',' | ';' | ':' | '(' | ')' | '[' | ']' | '"' | '\'' | '`' | '!' | '?'
            )
    })
    .map(|token| token.strip_suffix('.').unwrap_or(token))
    .filter(|token| !token.is_empty())
}

fn is_package_token(token: &str, name: &str) -> bool {
    if token.eq_ignore_ascii_case(name) {
        return true;
    }
    let Some(head) = token.get(..name.len()) else {
        return false;
    };
    let Some(rest) = token[name.len()..].strip_prefix('-') else {
        return false;
    };
    head.eq_ignore_ascii_case(name) && rest.starts_with(|c: char| c.is_ascii_digit())
}

pub fn is_intervention(title: &str) -> bool {
    let lower = title.to_ascii_lowercase();
    lower.contains("manual intervention") || lower.contains("breaking change")
}

pub fn is_aur_notice(title: &str) -> bool {
    tokens(title).any(|token| token.eq_ignore_ascii_case("AUR"))
}

#[cfg(test)]
mod tests {
    use super::*;

    const FEED: &str = r##"<?xml version="1.0" encoding="utf-8"?>
<rss version="2.0" xmlns:atom="http://www.w3.org/2005/Atom"><channel><title>Arch Linux: Recent news updates</title><link>https://archlinux.org/news/</link>
<item><title>virtualbox-ext-vnc &gt;= 7.2.12-2 requires manual intervention</title><link>https://archlinux.org/news/virtualbox-ext-vnc-7212-2-requires-manual-intervention/</link><description>&lt;p&gt;Previously, we installed its contents in a way that made &lt;code&gt;pacman&lt;/code&gt; not
aware of the files. To mitigate issues, use one of the following methods:&lt;/p&gt;
&lt;ul&gt;
&lt;li&gt;Uninstall &lt;code&gt;virtualbox-ext-vnc&lt;/code&gt; before upgrading the system, then
installing it again.&lt;/li&gt;
&lt;li&gt;Run &lt;code&gt;VBoxManage extpack uninstall &#39;VNC&#39;&lt;/code&gt; as root. See &lt;a href=&quot;https://archlinux.org/news/&quot;&gt;the news page&lt;/a&gt;.&lt;/li&gt;
&lt;/ul&gt;
&lt;p&gt;Reboot afterwards.&lt;/p&gt;</description><pubDate>Tue, 22 Sep 2026 09:09:27 +0000</pubDate><guid isPermaLink="false">tag:archlinux.org,2026-09-22:/news/virtualbox-ext-vnc-7212-2-requires-manual-intervention/</guid></item>
<item><title><![CDATA[Active AUR malicious packages incident &amp; more]]></title><link>https://archlinux.org/news/active-aur-malicious-packages-incident/</link><description><![CDATA[<p>Review <a href="https://example.org/pkgbuild">your</a> PKGBUILDs &amp; packages.</p>]]></description><pubDate>Tue, 21 Jul 2026 13:01:46 +0000</pubDate></item>
<item><title>Sin fecha</title><link>https://archlinux.org/news/sin-fecha/</link><description>&lt;p&gt;Esta entrada no tiene pubDate.&lt;/p&gt;</description></item>
</channel></rss>
"##;

    fn is_date_shape(text: &str) -> bool {
        text.len() == 10
            && text.bytes().enumerate().all(|(i, b)| match i {
                4 | 7 => b == b'-',
                _ => b.is_ascii_digit(),
            })
    }

    #[test]
    fn parse_rss_decodes_escaped_fields() {
        let items = parse_rss(FEED);
        assert_eq!(items.len(), 2, "el item sin pubDate se descarta");
        let first = &items[0];
        assert_eq!(
            first.title,
            "virtualbox-ext-vnc >= 7.2.12-2 requires manual intervention"
        );
        assert_eq!(
            first.link,
            "https://archlinux.org/news/virtualbox-ext-vnc-7212-2-requires-manual-intervention/"
        );
        assert_eq!(first.published, 1790068167);
        assert!(is_date_shape(&first.date), "fecha: {}", first.date);
        assert_eq!(Some(first.date.clone()), local_date(first.published));
        assert_eq!(
            first.body,
            "Previously, we installed its contents in a way that made pacman not aware of the \
             files. To mitigate issues, use one of the following methods:\n\
             • Uninstall virtualbox-ext-vnc before upgrading the system, then installing it again.\n\
             • Run VBoxManage extpack uninstall 'VNC' as root. See the news page.\n\
             Reboot afterwards."
        );
    }

    #[test]
    fn parse_rss_reads_cdata_literally_but_decodes_its_html() {
        let items = parse_rss(FEED);
        let second = &items[1];
        assert_eq!(
            second.title,
            "Active AUR malicious packages incident &amp; more"
        );
        assert_eq!(second.published, 1784638906);
        assert_eq!(second.body, "Review your PKGBUILDs & packages.");
    }

    #[test]
    fn html_to_text_breaks_lines_and_drops_tags() {
        assert_eq!(html_to_text("<p>a</p><p>b</p>"), "a\nb");
        assert_eq!(html_to_text("a<br>b<br/>c<BR />d"), "a\nb\nc\nd");
        assert_eq!(
            html_to_text(
                r#"<a href="https://x.example/?a=1&amp;b=2">enlace</a> y <b class="x">negrita</b>"#
            ),
            "enlace y negrita"
        );
    }

    #[test]
    fn html_to_text_decodes_entities_once() {
        assert_eq!(html_to_text("&amp;lt;"), "&lt;");
        assert_eq!(html_to_text("Tom &amp; Jerry"), "Tom & Jerry");
        assert_eq!(html_to_text("a < b"), "a < b");
    }

    #[test]
    fn html_to_text_turns_list_items_into_bullets() {
        assert_eq!(
            html_to_text("<ul><li>uno</li><li>dos</li></ul>"),
            "• uno\n• dos"
        );
        assert_eq!(html_to_text("<UL><LI>uno</LI></UL>"), "• uno");
        assert_eq!(
            html_to_text("texto<ul><li>a</li><li>b</li></ul>"),
            "texto\n• a\n• b"
        );
        assert_eq!(
            html_to_text("<p>Opciones:</p>\n<ol>\n<li>uno</li>\n</ol>"),
            "Opciones:\n• uno"
        );
    }

    #[test]
    fn html_to_text_keeps_pre_lines_and_spaces() {
        assert_eq!(
            html_to_text(
                "<p>Ejecutá:</p><pre><code>pacman -Rdd linux-firmware\npacman -Syu linux-firmware\n</code></pre>"
            ),
            "Ejecutá:\npacman -Rdd linux-firmware\npacman -Syu linux-firmware"
        );
        assert_eq!(html_to_text("<pre>a    b &lt; c</pre>"), "a    b < c");
    }

    #[test]
    fn decode_entities_known_numeric_and_unknown() {
        assert_eq!(decode_entities("&amp;&lt;&gt;&quot;&apos;"), "&<>\"'");
        assert_eq!(decode_entities("&#39;&#x27;&#233;"), "''é");
        assert_eq!(decode_entities("&foo; & fin"), "&foo; & fin");
        assert_eq!(
            decode_entities("&ñandú; &#xZZ; &#; &#0;"),
            "&ñandú; &#xZZ; &#; &#0;"
        );
    }

    const LOG: &str = "\
[2026-09-01T10:00:00-0500] [PACMAN] Running 'pacman -Syu'
[2026-09-01T10:00:01-0500] [PACMAN] starting full system upgrade
[2026-09-01T10:00:09-0500] [ALPM] upgraded foo (1.0-1 -> 1.1-1)
[2026-10-17T11:05:47-0500] [PACMAN] starting full system upgrade
[2026-10-17T11:06:02-0500] [PACMAN] Running 'pacman -S bar'
[2019-01-01 12:00] [PACMAN] starting full system upgrade
";

    #[test]
    fn last_upgrade_takes_the_last_line_with_a_valid_timestamp() {
        // 2026-10-17T11:05:47-0500; la línea de formato viejo del final se ignora.
        assert_eq!(last_upgrade_from(LOG), Some(1792253147));
        assert_eq!(last_upgrade_from(""), None);
        assert_eq!(
            last_upgrade_from("[2019-01-01 12:00] [PACMAN] starting full system upgrade\n"),
            None
        );
    }

    #[test]
    fn mentions_package_matches_name_tokens() {
        assert!(mentions_package(
            "Mkinitcpio >=42 requires manual intervention for TPM2-based unlocking of LUKS devices",
            "mkinitcpio"
        ));
        let vbox = "virtualbox-ext-vnc >= 7.2.12-2 requires manual intervention";
        assert!(mentions_package(vbox, "virtualbox-ext-vnc"));
        assert!(!mentions_package(vbox, "virtualbox"));
        assert!(!mentions_package(
            "Arch Linux 2026 Leader Election Results",
            "linux"
        ));
        assert!(mentions_package(
            "The sshd service needs to be restarted after upgrading to openssh-9.8p1",
            "openssh"
        ));
        let varnish =
            "Breaking changes for all users of `varnish`, which is renamed to `vinyl-cache`";
        assert!(mentions_package(varnish, "varnish"));
        assert!(mentions_package(varnish, "vinyl-cache"));
        assert!(mentions_package(
            "kea >= 1:3.0.3-6 update requires manual intervention",
            "kea"
        ));
        assert!(!mentions_package(
            ".NET packages may require manual intervention",
            "dotnet-runtime"
        ));
        assert!(!mentions_package("Foo v bar", "v"));
    }

    #[test]
    fn title_flags_for_interventions_and_aur_notices() {
        assert!(is_intervention(
            "Mkinitcpio >=42 requires manual intervention for TPM2-based unlocking of LUKS devices"
        ));
        assert!(is_intervention(
            "Breaking changes for all users of `varnish`, which is renamed to `vinyl-cache`"
        ));
        assert!(!is_intervention("iptables now defaults to the nft backend"));

        assert!(is_aur_notice("Active AUR malicious packages incident"));
        assert!(is_aur_notice("aur helpers"));
        assert!(!is_aur_notice("Authorization changes"));
    }

    #[test]
    #[ignore = "red"]
    fn fetch_returns_items_online() {
        let items = fetch().expect("el feed de noticias responde");
        assert!(items.iter().any(|item| !item.title.is_empty()));
    }
}
