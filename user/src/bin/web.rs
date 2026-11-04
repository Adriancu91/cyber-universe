//! web: the text browser (P6-53, C-68 F6a). Downloads a page over HTTPS (or plain HTTP) and shows it as text: the
//! title, headings, paragraphs, lists, tables and numbered links `[N]` (user/src/html.rs). Pages built by JavaScript
//! do not show: this is step (a), simple pages and documentation.
//!   web <adresa>        shows the first screen page of https://... or http://... (a bare name gets https://)
//!   web <N>             opens link N of the last page shown
//!   web --pagina <P>    screen page P of the last page (without downloading it again)
//!   web --legaturi      all the links of the last page, with their full address
//! Options: --linii L (lines per screen page, default 40), --latime W (characters per line, default 78),
//! --adresa A (connect to A instead of resolving the name — tests only). The last page is kept in /tmp/web/
//! (pagina.txt = the text, legaturi.txt = its address, then one link per line), so `cat` can show it too.
//! Redirects are followed (at most 5). Exit code 0 if the page was shown, 1 otherwise.
#![no_std]
#![no_main]

extern crate alloc;

use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;
use cyber_user::net::{TcpStream, parse_ip, resolve};
use cyber_user::{error_text, println};

cyber_user::entry!(main);
cyber_user::heap!(8 * 1024 * 1024);

#[path = "../http_client.rs"]
mod http_client;
#[path = "../html.rs"]
mod html;

/// The largest page kept (simple pages and documentation are far smaller).
const MAX_BODY: usize = 2 * 1024 * 1024;
const DIR: &str = "/tmp/web";
const TEXT: &str = "/tmp/web/pagina.txt";
const LINKS: &str = "/tmp/web/legaturi.txt";

struct Options {
    lines: usize,
    width: usize,
    address: Option<String>,
}

fn main() -> i64 {
    let mut opts = Options { lines: 40, width: 78, address: None };
    let mut target: Option<String> = None;
    let mut page: Option<usize> = None;
    let mut list_links = false;
    let mut args = cyber_user::args().skip(1);
    while let Some(a) = args.next() {
        let mut number = |what: &str| -> Option<usize> {
            let v = args.next().and_then(|v| v.parse::<usize>().ok()).filter(|&v| v > 0);
            if v.is_none() {
                println!("web: {} cere un numar mai mare ca 0", what);
            }
            v
        };
        match a {
            "--linii" => match number("--linii") {
                Some(v) => opts.lines = v,
                None => return 1,
            },
            "--latime" => match number("--latime") {
                Some(v) => opts.width = v.clamp(20, 200),
                None => return 1,
            },
            "--pagina" => match number("--pagina") {
                Some(v) => page = Some(v),
                None => return 1,
            },
            "--adresa" => match args.next() {
                Some(v) => opts.address = Some(v.to_string()),
                None => {
                    println!("web: --adresa cere o adresa IP");
                    return 1;
                }
            },
            "--legaturi" => list_links = true,
            _ if target.is_none() && !a.starts_with("--") => target = Some(a.to_string()),
            _ => {
                println!("web: nu inteleg \"{}\"", a);
                return usage();
            }
        }
    }
    if list_links {
        return show_links();
    }
    let Some(target) = target else {
        return match page {
            Some(p) => show_saved(p, &opts),
            None => usage(),
        };
    };
    let url = if let Ok(n) = target.parse::<usize>() {
        match saved_link(n) {
            Some(u) => u,
            None => return 1,
        }
    } else if target.starts_with("https://") || target.starts_with("http://") {
        target
    } else {
        format!("https://{}", target)
    };
    open_page(&url, page.unwrap_or(1), &opts)
}

fn usage() -> i64 {
    println!("folosire: web <adresa> | web <N> (legatura N) | web --pagina <P> | web --legaturi");
    println!("          [--linii L] [--latime W]");
    1
}

/// Downloads `url` (following redirects), turns it into text, keeps it in /tmp/web and shows screen page `page`.
fn open_page(url: &str, page: usize, opts: &Options) -> i64 {
    let mut url = url.to_string();
    let mut redirects = 0;
    let body = loop {
        println!("web: deschid {}", url);
        let resp = match get(&url, opts.address.as_deref()) {
            Ok(r) => r,
            Err(e) => {
                println!("web: {}", e);
                return 1;
            }
        };
        match (resp.status, resp.location) {
            (200, _) => break resp.body,
            (301 | 302 | 303 | 307 | 308, Some(loc)) if redirects < 5 => {
                let Some(next) = html::resolve(&url, &loc) else {
                    println!("web: redirectionare spre o adresa pe care nu o pot deschide: {}", loc);
                    return 1;
                };
                println!("web: redirectionat ({}) spre {}", resp.status, next);
                url = next;
                redirects += 1;
            }
            (301 | 302 | 303 | 307 | 308, _) => {
                println!("web: prea multe redirectionari");
                return 1;
            }
            (s, _) => {
                println!("web: serverul a raspuns {} (pagina nu se poate arata)", s);
                return 1;
            }
        }
    };
    let text = String::from_utf8_lossy(&body);
    let doc = if looks_like_html(&text) {
        html::render(&text, opts.width)
    } else {
        html::Doc { title: String::new(), lines: text.lines().map(|l| l.to_string()).collect(), links: Vec::new() }
    };
    let mut links = Vec::new();
    for l in &doc.links {
        links.push(html::resolve(&url, l).unwrap_or_else(|| l.clone()));
    }
    save(&url, &doc, &links);
    println!("web: {} octeti, {} randuri, {} legaturi", body.len(), doc.lines.len(), links.len());
    show(&doc.title, &doc.lines, &links, page, opts.lines)
}

/// Shows screen page `page` of the text; the last screen page ends with the list of links.
fn show(title: &str, lines: &[String], links: &[String], page: usize, per_page: usize) -> i64 {
    let pages = lines.len().div_ceil(per_page).max(1);
    if page > pages {
        println!("web: pagina are doar {} pagini de ecran", pages);
        return 1;
    }
    if !title.is_empty() {
        println!("== {} ==", title);
    }
    for l in lines.iter().skip((page - 1) * per_page).take(per_page) {
        println!("{}", l);
    }
    if page < pages {
        println!("-- pagina {} din {}; mai departe: web --pagina {}", page, pages, page + 1);
    } else {
        if !links.is_empty() {
            println!("-- legaturi:");
            for (i, l) in links.iter().enumerate() {
                println!("[{}] {}", i + 1, l);
            }
        }
        println!("-- sfarsit (pagina {} din {}); deschide o legatura cu: web <N>", page, pages);
    }
    0
}

fn looks_like_html(text: &str) -> bool {
    let head: String = text.chars().take(1024).collect::<String>().to_ascii_lowercase();
    ["<!doctype html", "<html", "<head", "<body", "<p>", "<h1", "<div", "<title"].iter().any(|t| head.contains(t))
}

/// GET over HTTPS (http_client) or plain HTTP (HTTP/1.0, read until the server closes).
fn get(url: &str, address: Option<&str>) -> Result<http_client::Response, String> {
    if url.starts_with("https://") {
        let (host, port, path) = http_client::split_url(url).ok_or("adresa de neinteles")?;
        return http_client::request("web", address, &host, port, "GET", &path, "Accept: text/html, text/plain\r\n", &[], MAX_BODY);
    }
    let rest = url.strip_prefix("http://").ok_or("doar adrese https:// si http://")?;
    let (authority, path) = match rest.find('/') {
        Some(i) => (&rest[..i], &rest[i..]),
        None => (rest, "/"),
    };
    let (host, port) = match authority.rsplit_once(':') {
        Some((h, p)) => (h, p.parse::<u16>().map_err(|_| "port invalid")?),
        None => (authority, 80),
    };
    let ip = match address {
        Some(a) => parse_ip(a).ok_or("adresa IP invalida")?,
        None => resolve(host).map_err(|e| format!("{}: {}", host, error_text(e)))?,
    };
    let mut conn = TcpStream::connect(ip, port, 5000).map_err(|e| format!("{}:{}: {}", host, port, error_text(e)))?;
    let req = format!("GET {} HTTP/1.0\r\nHost: {}\r\nUser-Agent: CyberUniverse-web\r\nAccept: text/html, text/plain\r\nConnection: close\r\n\r\n", path, host);
    conn.write_all(req.as_bytes()).map_err(|e| format!("trimitere: {}", error_text(e)))?;
    let mut raw = Vec::new();
    let mut buf = [0u8; 4096];
    loop {
        match conn.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => raw.extend_from_slice(&buf[..n]),
            Err(e) => return Err(format!("primire: {}", error_text(e))),
        }
        if raw.len() > MAX_BODY + 64 * 1024 {
            return Err(format!("raspuns prea mare (peste {} KiB)", MAX_BODY >> 10));
        }
    }
    let end = raw.windows(4).position(|w| w == b"\r\n\r\n").ok_or("raspuns HTTP de neinteles")?;
    let head = String::from_utf8_lossy(&raw[..end]).into_owned();
    let mut lines = head.split("\r\n");
    let status = lines.next().and_then(|l| l.split(' ').nth(1)).and_then(|s| s.parse().ok()).ok_or("raspuns HTTP fara cod")?;
    let location = lines.find_map(|l| {
        let (k, v) = l.split_once(':')?;
        k.trim().eq_ignore_ascii_case("location").then(|| v.trim().to_string())
    });
    raw.drain(..end + 4);
    Ok(http_client::Response { status, location, body: raw })
}

fn write_file(path: &str, data: &[u8]) -> bool {
    let r = cyber_user::remove(path);
    let r = if r < 0 && r != -3 { r } else { cyber_user::create(path, false) };
    let fd = if r < 0 { r } else { cyber_user::open(path) };
    if fd < 0 {
        return false;
    }
    let mut done = 0;
    while done < data.len() {
        let n = cyber_user::write(fd as u64, &data[done..data.len().min(done + 16 * 1024)]);
        if n <= 0 {
            break;
        }
        done += n as usize;
    }
    cyber_user::close(fd);
    done == data.len()
}

/// Keeps the page for `web <N>`, `web --pagina P` and `web --legaturi`: pagina.txt = title line, then the text;
/// legaturi.txt = the page's address, then one link per line.
fn save(url: &str, doc: &html::Doc, links: &[String]) {
    let _ = cyber_user::create("/tmp", true);
    let _ = cyber_user::create(DIR, true);
    let mut text = String::new();
    text.push_str(&doc.title);
    text.push('\n');
    for l in &doc.lines {
        text.push_str(l);
        text.push('\n');
    }
    let mut ls = String::from(url);
    ls.push('\n');
    for l in links {
        ls.push_str(l);
        ls.push('\n');
    }
    if !write_file(TEXT, text.as_bytes()) || !write_file(LINKS, ls.as_bytes()) {
        println!("web: nu am putut pastra pagina in {} (web <N> nu va merge)", DIR);
    }
}

fn read_saved(path: &str) -> Option<String> {
    let data = http_client::read_file(path, MAX_BODY * 2)?;
    Some(String::from_utf8_lossy(&data).into_owned())
}

fn saved_link(n: usize) -> Option<String> {
    let Some(all) = read_saved(LINKS) else {
        println!("web: nicio pagina deschisa inca (web <adresa>)");
        return None;
    };
    let link = all.lines().skip(1).nth(n.checked_sub(1)?).map(|l| l.to_string());
    if link.is_none() {
        println!("web: pagina are doar {} legaturi", all.lines().count().saturating_sub(1));
    }
    link
}

fn show_saved(page: usize, opts: &Options) -> i64 {
    let (Some(text), Some(links)) = (read_saved(TEXT), read_saved(LINKS)) else {
        println!("web: nicio pagina deschisa inca (web <adresa>)");
        return 1;
    };
    let mut it = text.lines();
    let title = it.next().unwrap_or("").to_string();
    let lines: Vec<String> = it.map(|l| l.to_string()).collect();
    let mut li = links.lines();
    let url = li.next().unwrap_or("");
    let links: Vec<String> = li.map(|l| l.to_string()).collect();
    println!("web: {} (pastrata)", url);
    show(&title, &lines, &links, page, opts.lines)
}

fn show_links() -> i64 {
    let Some(all) = read_saved(LINKS) else {
        println!("web: nicio pagina deschisa inca (web <adresa>)");
        return 1;
    };
    let mut it = all.lines();
    println!("web: legaturile din {}", it.next().unwrap_or(""));
    for (i, l) in it.enumerate() {
        println!("[{}] {}", i + 1, l);
    }
    0
}
