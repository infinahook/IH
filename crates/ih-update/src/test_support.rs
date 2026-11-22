//! Outils de test partagés : petit serveur HTTP local, clés et manifestes.
//!
//! Aucun test ne contacte le réseau : le serveur écoute sur `127.0.0.1` avec un
//! port éphémère et ne sert que les routes déclarées.

use ed25519_dalek::{Signer, SigningKey};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::io::{Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::Duration;

pub struct TestServer {
    address: SocketAddr,
    routes: Arc<Mutex<HashMap<String, Vec<u8>>>>,
    shutdown: Arc<AtomicBool>,
    handle: Option<JoinHandle<()>>,
}

impl TestServer {
    pub fn new(initial: Vec<(&str, Vec<u8>)>) -> Self {
        let listener = TcpListener::bind(("127.0.0.1", 0)).expect("écoute locale de test");
        listener
            .set_nonblocking(true)
            .expect("serveur de test non bloquant");
        let address = listener.local_addr().expect("adresse locale de test");
        let routes: HashMap<String, Vec<u8>> = initial
            .into_iter()
            .map(|(path, body)| (path.to_string(), body))
            .collect();
        let routes = Arc::new(Mutex::new(routes));
        let shutdown = Arc::new(AtomicBool::new(false));

        let thread_routes = Arc::clone(&routes);
        let thread_shutdown = Arc::clone(&shutdown);
        let handle = thread::spawn(move || {
            while !thread_shutdown.load(Ordering::SeqCst) {
                match listener.accept() {
                    Ok((stream, _)) => handle_connection(stream, &thread_routes),
                    Err(err) if err.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(2));
                    }
                    Err(_) => break,
                }
            }
        });

        Self {
            address,
            routes,
            shutdown,
            handle: Some(handle),
        }
    }

    pub fn set_route(&self, path: &str, body: Vec<u8>) {
        self.routes
            .lock()
            .expect("routes de test")
            .insert(path.to_string(), body);
    }

    pub fn url(&self, path: &str) -> String {
        format!("http://{}{}", self.address, path)
    }
}

impl Drop for TestServer {
    fn drop(&mut self) {
        self.shutdown.store(true, Ordering::SeqCst);
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

fn handle_connection(mut stream: TcpStream, routes: &Arc<Mutex<HashMap<String, Vec<u8>>>>) {
    let _ = stream.set_read_timeout(Some(Duration::from_secs(5)));
    let mut request = Vec::new();
    let mut buffer = [0u8; 1024];
    while request.len() < 64 * 1024 {
        match stream.read(&mut buffer) {
            Ok(0) => break,
            Ok(read) => {
                request.extend_from_slice(&buffer[..read]);
                if request.windows(4).any(|window| window == b"\r\n\r\n") {
                    break;
                }
            }
            Err(_) => return,
        }
    }

    let text = String::from_utf8_lossy(&request);
    let path = text
        .lines()
        .next()
        .and_then(|line| line.split_whitespace().nth(1))
        .unwrap_or("/")
        .to_string();

    let body = routes.lock().expect("routes de test").get(&path).cloned();

    let response = match body {
        Some(body) => {
            let mut response = format!(
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nContent-Type: application/octet-stream\r\nConnection: close\r\n\r\n",
                body.len()
            )
            .into_bytes();
            response.extend_from_slice(&body);
            response
        }
        None => {
            b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".to_vec()
        }
    };

    let _ = stream.write_all(&response);
    let _ = stream.flush();
}

pub fn sha256_hex(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

pub fn test_signing_key() -> SigningKey {
    SigningKey::from_bytes(&[42u8; 32])
}

pub fn sign_hex(key: &SigningKey, bytes: &[u8]) -> String {
    hex::encode(key.sign(bytes).to_bytes())
}

pub fn public_key_hex(key: &SigningKey) -> String {
    hex::encode(key.verifying_key().to_bytes())
}

pub fn manifest_json(version: &str, artifact_url: &str, sha256: &str) -> Vec<u8> {
    serde_json::json!({
        "schema": 1,
        "version": version,
        "channel": "stable",
        "notes": "notes de test",
        "released_at": "2026-11-22T10:00:00Z",
        "platforms": {
            "linux-x86_64-appimage": {
                "url": artifact_url,
                "sha256": sha256,
                "kind": "appimage"
            }
        }
    })
    .to_string()
    .into_bytes()
}
