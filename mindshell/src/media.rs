//! Per-process loopback media broker. GStreamer only accepts built-in URI
//! schemes and cannot read arbitrary files through WebKit's sandbox. An
//! unguessable capability exposes only each explicitly selected media file.
use std::collections::HashMap;
use std::io::{BufRead, BufReader, Read, Seek, SeekFrom, Write};
use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
use std::sync::{Arc, Mutex, OnceLock};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

struct Server { port: u16, files: Arc<Mutex<HashMap<String, (PathBuf, &'static str)>>> }
static SERVER: OnceLock<Result<Server, String>> = OnceLock::new();

pub fn open(path: &str) -> Result<String, String> {
    let path = crate::fs::expand(path).canonicalize().map_err(|_| "Video file not found")?;
    if !path.is_file() { return Err("Choose a regular media file".into()); }
    let mime = match path.extension().and_then(|s| s.to_str()).unwrap_or("").to_ascii_lowercase().as_str() {
        "webm" => "video/webm", "mp4" | "m4v" => "video/mp4", "mov" => "video/quicktime",
        "mkv" => "video/x-matroska", "ogv" => "video/ogg", "mp3" => "audio/mpeg",
        "ogg" | "oga" => "audio/ogg", "wav" => "audio/wav", _ => return Err("Unsupported media type".into()),
    };
    let server = SERVER.get_or_init(start).as_ref().map_err(Clone::clone)?;
    let token = gtk4::glib::uuid_string_random().to_string();
    let mut files = server.files.lock().map_err(|_| "Media registry unavailable")?;
    if files.len() >= 64 { files.clear(); }
    files.insert(token.clone(), (path, mime));
    Ok(format!("http://127.0.0.1:{}/{token}", server.port))
}

fn start() -> Result<Server, String> {
    let listener = TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0)).map_err(|e| e.to_string())?;
    let port = listener.local_addr().map_err(|e| e.to_string())?.port();
    let files = Arc::new(Mutex::new(HashMap::new()));
    let registry = files.clone();
    std::thread::Builder::new().name("mindos-media".into()).spawn(move || {
        let active = Arc::new(AtomicUsize::new(0));
        for stream in listener.incoming().flatten() {
            if active.load(Ordering::Relaxed) >= 8 { continue; }
            active.fetch_add(1, Ordering::Relaxed);
            let active = active.clone(); let registry = registry.clone();
            std::thread::spawn(move || {
                let _ = serve(stream, registry);
                active.fetch_sub(1, Ordering::Relaxed);
            });
        }
    }).map_err(|e| e.to_string())?;
    Ok(Server { port, files })
}

fn serve(mut stream: TcpStream, files: Arc<Mutex<HashMap<String, (PathBuf, &'static str)>>>) -> std::io::Result<()> {
    stream.set_read_timeout(Some(Duration::from_secs(5)))?;
    stream.set_write_timeout(Some(Duration::from_secs(15)))?;
    let mut reader = BufReader::new((&stream).take(16384));
    let mut first = String::new(); reader.read_line(&mut first)?;
    let mut parts = first.split_whitespace();
    let method = parts.next().unwrap_or(""); let token = parts.next().unwrap_or("").trim_start_matches('/');
    if !["GET", "HEAD"].contains(&method) { return stream.write_all(b"HTTP/1.1 405 Method Not Allowed\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"); }
    let mut range = None;
    loop {
        let mut line = String::new();
        if reader.read_line(&mut line)? == 0 || line == "\r\n" { break; }
        if let Some((name, value)) = line.split_once(':') {
            if name.eq_ignore_ascii_case("Range") { range = Some(value.trim().to_owned()); }
        }
    }
    let file = files.lock().ok().and_then(|files| files.get(token).cloned());
    let Some((path, mime)) = file else { return stream.write_all(b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"); };
    let mut file = match std::fs::File::open(path) { Ok(file) => file, Err(_) => return stream.write_all(b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n") };
    let size = file.metadata()?.len();
    let Some((start,end)) = crate::scheme::media_range(range.as_deref(),size) else {
        return write!(stream,"HTTP/1.1 416 Range Not Satisfiable\r\nContent-Range: bytes */{size}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
    };
    let status = if range.is_some() { "206 Partial Content" } else { "200 OK" };
    write!(stream,"HTTP/1.1 {status}\r\nContent-Type: {mime}\r\nContent-Length: {}\r\nAccept-Ranges: bytes\r\nAccess-Control-Allow-Origin: *\r\nCache-Control: no-store\r\nConnection: close\r\n",end-start+1)?;
    if range.is_some() { write!(stream,"Content-Range: bytes {start}-{end}/{size}\r\n")?; }
    stream.write_all(b"\r\n")?;
    if method == "GET" { file.seek(SeekFrom::Start(start))?; std::io::copy(&mut file.take(end-start+1), &mut stream)?; }
    Ok(())
}

#[cfg(test)]
mod tests {
    #[test]
    fn capability_broker_limits_ranges_and_refuses_unknown_paths() {
        use super::*;
        let dir=std::env::temp_dir().join(gtk4::glib::uuid_string_random().as_str());
        std::fs::create_dir(&dir).unwrap(); let path=dir.join("test.webm"); std::fs::write(&path,b"0123456789").unwrap();
        let uri=open(path.to_str().unwrap()).unwrap();
        let (host,token)=uri.strip_prefix("http://").unwrap().split_once('/').unwrap();
        let request=|token:&str,range:&str| { let mut s=TcpStream::connect(host).unwrap(); s.write_all(format!("GET /{token} HTTP/1.1\r\nHost: localhost\r\nRange: {range}\r\n\r\n").as_bytes()).unwrap(); let mut bytes=String::new(); s.read_to_string(&mut bytes).unwrap(); bytes };
        let result=request(token,"bytes=2-4"); assert!(result.starts_with("HTTP/1.1 206")); assert!(result.ends_with("\r\n\r\n234"));
        assert!(request("../test.webm","bytes=0-").starts_with("HTTP/1.1 404"));
        assert!(request(token,"bytes=100-").starts_with("HTTP/1.1 416"));
        std::fs::remove_dir_all(dir).unwrap();
    }
}
