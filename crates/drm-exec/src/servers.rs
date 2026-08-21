//! In-process loopback fixture servers that back the `http.request` and
//! `ipc.request` capabilities. They exist so those capabilities have a real
//! socket round-trip to perform -- real `connect()`/`accept()`, real bytes
//! on the wire -- without requiring genuine external network access
//! (useful in network-restricted deployments, and deterministic in tests).

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::Duration;

pub struct TcpFixtureServer {
    pub port: u16,
    stop: Arc<AtomicBool>,
    handle: Option<JoinHandle<()>>,
}

impl TcpFixtureServer {
    pub fn start() -> std::io::Result<Self> {
        let listener = TcpListener::bind("127.0.0.1:0")?;
        let port = listener.local_addr()?.port();
        listener.set_nonblocking(true)?;
        let stop = Arc::new(AtomicBool::new(false));
        let stop2 = stop.clone();
        let handle = thread::spawn(move || {
            while !stop2.load(Ordering::Relaxed) {
                match listener.accept() {
                    Ok((mut stream, _)) => {
                        let mut buf = [0u8; 2048];
                        let n = stream.read(&mut buf).unwrap_or(0);
                        let req = String::from_utf8_lossy(&buf[..n]);
                        let path = req.split_whitespace().nth(1).unwrap_or("/news_0.html");
                        let idx: usize = path
                            .trim_matches('/')
                            .trim_start_matches("news_")
                            .trim_end_matches(".html")
                            .parse()
                            .unwrap_or(0);
                        let mut body = format!("<html><body><h1>News {idx}</h1><p>");
                        for j in 1..36 {
                            body.push_str(&format!("Story{idx}-{j} DRM local scheduling repeated task optimization Linux "));
                        }
                        body.push_str("</p></body></html>");
                        let resp = format!(
                            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                            body.len(),
                            body
                        );
                        let _ = stream.write_all(resp.as_bytes());
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(2));
                    }
                    Err(_) => break,
                }
            }
        });
        Ok(Self {
            port,
            stop,
            handle: Some(handle),
        })
    }

    pub fn get(&self, path: &str) -> std::io::Result<String> {
        let mut stream = TcpStream::connect(("127.0.0.1", self.port))?;
        let req = format!("GET {path} HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n");
        stream.write_all(req.as_bytes())?;
        let mut resp = String::new();
        stream.read_to_string(&mut resp)?;
        Ok(resp.split("\r\n\r\n").nth(1).unwrap_or("").to_string())
    }
}

impl Drop for TcpFixtureServer {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        let _ = TcpStream::connect(("127.0.0.1", self.port));
        if let Some(h) = self.handle.take() {
            let _ = h.join();
        }
    }
}

pub struct UnixFixtureServer {
    pub path: PathBuf,
    stop: Arc<AtomicBool>,
    handle: Option<JoinHandle<()>>,
}

impl UnixFixtureServer {
    pub fn start(path: &Path) -> std::io::Result<Self> {
        let _ = std::fs::remove_file(path);
        let listener = UnixListener::bind(path)?;
        listener.set_nonblocking(true)?;
        let stop = Arc::new(AtomicBool::new(false));
        let stop2 = stop.clone();
        let path_owned = path.to_path_buf();
        let handle = thread::spawn(move || {
            while !stop2.load(Ordering::Relaxed) {
                match listener.accept() {
                    Ok((mut stream, _)) => {
                        let mut buf = [0u8; 1024];
                        let n = stream.read(&mut buf).unwrap_or(0);
                        let mut out = b"ipc-ok:".to_vec();
                        out.extend_from_slice(&buf[..n]);
                        let _ = stream.write_all(&out);
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(2));
                    }
                    Err(_) => break,
                }
            }
        });
        Ok(Self {
            path: path_owned,
            stop,
            handle: Some(handle),
        })
    }

    pub fn roundtrip(&self, payload: &str) -> std::io::Result<String> {
        let mut stream = UnixStream::connect(&self.path)?;
        stream.write_all(payload.as_bytes())?;
        let mut buf = [0u8; 1024];
        let n = stream.read(&mut buf)?;
        Ok(String::from_utf8_lossy(&buf[..n]).to_string())
    }
}

impl Drop for UnixFixtureServer {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        let _ = UnixStream::connect(&self.path);
        if let Some(h) = self.handle.take() {
            let _ = h.join();
        }
        let _ = std::fs::remove_file(&self.path);
    }
}
