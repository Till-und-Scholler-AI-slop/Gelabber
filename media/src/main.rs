use std::env;
use std::io::{Read, Write};
use std::net::TcpListener;

fn main() {
    let addr = env::var("MEDIA_ADDR").unwrap_or_else(|_| "0.0.0.0:8081".into());
    let listener = TcpListener::bind(&addr).unwrap_or_else(|err| {
        panic!("gelabber-media stub failed to bind {addr}: {err}");
    });
    eprintln!("gelabber-media stub listening on {addr}");

    for incoming in listener.incoming() {
        let Ok(mut stream) = incoming else {
            continue;
        };
        let mut buf = [0_u8; 2048];
        let _ = stream.read(&mut buf);
        let body = br#"{"service":"media","status":"stub"}"#;
        let header = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            body.len()
        );
        let _ = stream.write_all(header.as_bytes());
        let _ = stream.write_all(body);
    }
}
