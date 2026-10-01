#![allow(dead_code)]
use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::sync::mpsc::{channel, Receiver};

pub struct Captured {
    pub path: String,
    pub headers: Vec<(String, String)>,
    pub body: serde_json::Value,
}

impl Captured {
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    }
}

pub struct FakeServer {
    pub url: String,
    rx: Receiver<Captured>,
}

impl FakeServer {
    /// Serve one request: capture it and reply with `status` and the SSE `body`.
    pub fn once(status: u16, body: &'static str) -> FakeServer {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let (tx, rx) = channel();
        std::thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            let mut line = String::new();
            reader.read_line(&mut line).unwrap();
            let path = line.split_whitespace().nth(1).unwrap_or("").to_string();
            let mut headers = Vec::new();
            let mut len = 0usize;
            loop {
                line.clear();
                reader.read_line(&mut line).unwrap();
                let l = line.trim_end();
                if l.is_empty() {
                    break;
                }
                if let Some((k, v)) = l.split_once(':') {
                    if k.eq_ignore_ascii_case("content-length") {
                        len = v.trim().parse().unwrap();
                    }
                    headers.push((k.to_string(), v.trim().to_string()));
                }
            }
            let mut buf = vec![0; len];
            reader.read_exact(&mut buf).unwrap();
            let body_json = serde_json::from_slice(&buf).unwrap_or(serde_json::Value::Null);
            tx.send(Captured {
                path,
                headers,
                body: body_json,
            })
            .unwrap();
            let mut out = stream;
            write!(
                out,
                "HTTP/1.1 {status} X\r\ncontent-type: text/event-stream\r\nconnection: close\r\n\r\n{body}"
            )
            .unwrap();
        });
        FakeServer { url, rx }
    }

    pub fn request(&self) -> Captured {
        self.rx.recv().unwrap()
    }
}
