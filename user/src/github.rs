//! Shared by gh-trimite and gh-ia (P6-09, P6-52): /etc/github.conf, repository and path checks, URL encoding.
use alloc::format;
use alloc::string::{String, ToString};
use crate::http_client::read_file;
use cyber_user::println;

pub struct Config {
    pub host: String,
    pub port: u16,
    pub address: Option<String>,
    pub token: String,
    /// Where `gh-trimite --rapoarte` sends the stick's reports (C-84).
    pub hub: String,
    /// P6-220 (C-166 step 5): the OS's own code, where the agent reads and proposes (`cod = …`).
    #[allow(dead_code)]
    pub code: String,
}

pub fn config() -> Config {
    let mut c = Config { host: "api.github.com".to_string(), port: 443, address: None, token: "/usb/github.token".to_string(), hub: "Adriancu91/Cyber-hub".to_string(), code: "Adriancu91/cyber-universe".to_string() };
    let Some(text) = read_file("/etc/github.conf", 4096) else { return c };
    for line in core::str::from_utf8(&text).unwrap_or("").lines() {
        let line = line.trim();
        let Some((name, value)) = line.split_once('=') else { continue };
        let (name, value) = (name.trim(), value.trim().to_string());
        if line.starts_with('#') || value.is_empty() {
            continue;
        }
        match name {
            "host" => c.host = value,
            "port" => c.port = value.parse().unwrap_or(c.port),
            "address" => c.address = Some(value),
            "token" => c.token = value,
            "hub" => c.hub = value,
            "cod" => c.code = value,
            _ => println!("github: /etc/github.conf: nume necunoscut: {}", name),
        }
    }
    c
}

/// A path for the URL: unreserved characters and "/" stay, the rest (spaces, diacritics…) is %XX (RFC 3986).
pub fn encode_path(path: &str) -> String {
    let mut out = String::new();
    for b in path.bytes() {
        if b.is_ascii_alphanumeric() || b"-._~/".contains(&b) {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{:02X}", b));
        }
    }
    out
}

pub fn valid_repo(repo: &str) -> bool {
    let mut parts = repo.split('/');
    let ok = |p: Option<&str>| p.is_some_and(|p| !p.is_empty() && p.bytes().all(|b| b.is_ascii_alphanumeric() || b"-_.".contains(&b)));
    ok(parts.next()) && ok(parts.next()) && parts.next().is_none()
}

pub fn valid_path(path: &str) -> bool {
    !path.is_empty() && !path.starts_with('/') && !path.ends_with('/') && path.split('/').all(|p| !p.is_empty() && p != "." && p != "..")
}


/// P6-204: the token file, read once (the program asks for the right to see it; the caller gives up its secrets after).
#[allow(dead_code)]
pub fn read_token(c: &Config) -> Option<String> {
    let _ = cyber_user::rights::ask_secret(&c.token);
    crate::http_client::read_file(&c.token, 512).map(|t| String::from_utf8_lossy(&t).trim().to_string()).filter(|t| !t.is_empty())
}

/// P6-204 (C-166 step 3, the agent's live state): one file put on the default branch through the "contents" API, as
/// `gh-trimite` does it (GET: its sha, and nothing sent when it is already the same; then PUT). Returns "creat",
/// "actualizat" or "la fel".
#[allow(dead_code)]
pub fn put_contents(c: &Config, who: &str, token: &str, repo: &str, path: &str, data: &[u8], message: &str) -> Result<&'static str, String> {
    put_contents_on(c, who, token, repo, path, data, message, None)
}

fn headers(token: &str) -> String {
    format!("Accept: application/vnd.github+json\r\nX-GitHub-Api-Version: 2022-11-28\r\nAuthorization: Bearer {}\r\n", token)
}

fn fail(status: u16) -> String {
    match status {
        401 => "tokenul GitHub e gresit sau a expirat (401)".to_string(),
        403 => "GitHub a refuzat (403): fara drept de scriere sau limita de cereri".to_string(),
        404 => "depozitul nu exista sau tokenul nu are acces la el (404)".to_string(),
        409 => "fisierul s-a schimbat pe GitHub intre timp (409)".to_string(),
        422 => "GitHub a refuzat cererea (422)".to_string(),
        s => format!("eroare {} de la GitHub", s),
    }
}

/// P6-220: what `get_file` found: a file's bytes, or a folder's entries (name, `/` at the end for a folder).
#[allow(dead_code)]
pub enum Found {
    File(alloc::vec::Vec<u8>),
    Dir(alloc::vec::Vec<String>),
}

/// P6-220 (C-166 step 5): one file or folder of `repo` on branch `reference` (contents API, read only).
#[allow(dead_code)]
pub fn get_file(c: &Config, who: &str, token: &str, repo: &str, path: &str, reference: &str) -> Result<Found, String> {
    use cyber_json::{Value, base64};
    if !valid_repo(repo) || !(path.is_empty() || valid_path(path)) {
        return Err(format!("depozit sau cale invalida: {} {}", repo, path));
    }
    let api = format!("/repos/{}/contents/{}?ref={}", repo, encode_path(path), encode_path(reference));
    let r = crate::http_client::request(who, c.address.as_deref(), &c.host, c.port, "GET", &api, &headers(token), &[], 1536 * 1024)?;
    if r.status == 404 {
        return Err(format!("{} nu exista pe {} (sau tokenul nu are acces la {})", if path.is_empty() { "/" } else { path }, reference, repo));
    }
    if r.status != 200 {
        return Err(fail(r.status));
    }
    let j = core::str::from_utf8(&r.body).ok().and_then(|t| cyber_json::parse(t).ok()).ok_or("raspunsul GitHub nu e JSON")?;
    if let Some(list) = j.as_array() {
        let names = list
            .iter()
            .filter_map(|e| {
                let name = e.get("name").and_then(Value::as_str)?;
                Some(if e.get("type").and_then(Value::as_str) == Some("dir") { format!("{}/", name) } else { name.to_string() })
            })
            .collect();
        return Ok(Found::Dir(names));
    }
    let content = j.get("content").and_then(Value::as_str).ok_or("fisier fara continut (prea mare pentru API?)")?;
    base64::decode(content).map(Found::File).ok_or_else(|| "continut base64 gresit".to_string())
}

/// P6-220 (C-166 step 5): makes branch `b` from the tip of `main`. A branch that is already there is an error: a
/// proposal never writes over another one.
#[allow(dead_code)]
pub fn create_branch(c: &Config, who: &str, token: &str, repo: &str, b: &str) -> Result<(), String> {
    use cyber_json::Value;
    let h = headers(token);
    let r = crate::http_client::request(who, c.address.as_deref(), &c.host, c.port, "GET", &format!("/repos/{}/git/ref/heads/{}", repo, encode_path(b)), &h, &[], 64 * 1024)?;
    if r.status == 200 {
        return Err(format!("ramura {} exista deja; alege alt subiect", b));
    }
    let r = crate::http_client::request(who, c.address.as_deref(), &c.host, c.port, "GET", &format!("/repos/{}/git/ref/heads/main", repo), &h, &[], 64 * 1024)?;
    if r.status != 200 {
        return Err(format!("varful lui main: {}", fail(r.status)));
    }
    let j = core::str::from_utf8(&r.body).ok().and_then(|t| cyber_json::parse(t).ok()).ok_or("raspunsul GitHub nu e JSON")?;
    let sha = j.get("object").and_then(|o| o.get("sha")).and_then(Value::as_str).ok_or("raspuns fara sha")?.to_string();
    let body = Value::object([("ref", Value::String(format!("refs/heads/{}", b))), ("sha", Value::String(sha))]).to_string();
    let r = crate::http_client::request(who, c.address.as_deref(), &c.host, c.port, "POST", &format!("/repos/{}/git/refs", repo), &format!("{}Content-Type: application/json\r\n", h), body.as_bytes(), 64 * 1024)?;
    match r.status {
        201 => Ok(()),
        422 => Err(format!("ramura {} nu s-a putut face (exista deja?)", b)),
        s => Err(format!("ramura {}: {}", b, fail(s))),
    }
}

/// `put_contents` on `branch` (None = the default branch).
#[allow(dead_code)]
#[allow(clippy::too_many_arguments)]
pub fn put_contents_on(c: &Config, who: &str, token: &str, repo: &str, path: &str, data: &[u8], message: &str, branch: Option<&str>) -> Result<&'static str, String> {
    use cyber_json::{Value, base64};
    const MAX_ANSWER: usize = 512 * 1024;
    if !valid_repo(repo) || !valid_path(path) {
        return Err(format!("depozit sau cale invalida: {} {}", repo, path));
    }
    let api = format!("/repos/{}/contents/{}", repo, encode_path(path));
    let headers = headers(token);
    let get_api = match branch {
        Some(b) => format!("{}?ref={}", api, encode_path(b)),
        None => api.clone(),
    };
    let r = crate::http_client::request(who, c.address.as_deref(), &c.host, c.port, "GET", &get_api, &headers, &[], 1536 * 1024)?;
    let old_sha = match r.status {
        200 => {
            let j = core::str::from_utf8(&r.body).ok().and_then(|t| cyber_json::parse(t).ok()).ok_or("raspunsul GitHub nu e JSON")?;
            let sha = j.get("sha").and_then(Value::as_str).ok_or("raspuns fara sha (dosar?)")?.to_string();
            if j.get("content").and_then(Value::as_str).and_then(base64::decode).is_some_and(|old| old == *data) {
                return Ok("la fel");
            }
            Some(sha)
        }
        404 => None,
        s => return Err(fail(s)),
    };
    let mut fields = alloc::vec![(String::from("message"), Value::str(message)), (String::from("content"), Value::String(base64::encode(data)))];
    if let Some(sha) = &old_sha {
        fields.push((String::from("sha"), Value::str(sha)));
    }
    if let Some(b) = branch {
        fields.push((String::from("branch"), Value::str(b)));
    }
    let body = Value::Object(fields).to_string();
    let r = crate::http_client::request(who, c.address.as_deref(), &c.host, c.port, "PUT", &api, &format!("{}Content-Type: application/json\r\n", headers), body.as_bytes(), MAX_ANSWER)?;
    if r.status != 200 && r.status != 201 {
        return Err(fail(r.status));
    }
    Ok(if old_sha.is_some() { "actualizat" } else { "creat" })
}
