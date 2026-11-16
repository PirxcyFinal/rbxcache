//! Keeping to one window. Two copies would both follow the cache and write
//! over each other's replacements, so a second start hands over to the first.
//!
//! The first copy listens on a loopback port of its own user's. A later copy that finds the
//! port taken asks whoever holds it to show its window, and exits if the
//! answer is the one this app gives.

use std::io::{Read, Write};
use std::net::{Ipv4Addr, SocketAddr, TcpListener, TcpStream};
use std::sync::mpsc::Sender;
use std::time::Duration;

/// The first of the ports a copy may listen on, in a range no service is registered in.
const FIRST_PORT: u16 = 47_000;
const PORTS: u64 = 2_000;
const ASK: &[u8] = b"rbxcache show\n";
const ANSWER: &[u8] = b"rbxcache shown\n";

/// The port for this user's copy. It follows from the app's data folder, so
/// two people logged in to one computer do not shut each other out.
fn address() -> SocketAddr {
    let folder = crate::protection::data_dir().map(|dir| dir.to_string_lossy().into_owned()).unwrap_or_default();
    // FNV-1a: the same folder has to give the same port in every copy and every version.
    let hash = folder.bytes().fold(0xcbf2_9ce4_8422_2325_u64, |hash, byte| (hash ^ byte as u64).wrapping_mul(0x0000_0100_0000_01b3));
    SocketAddr::from((Ipv4Addr::LOCALHOST, FIRST_PORT + (hash % PORTS) as u16))
}

pub enum Claim {
    /// This is the only copy. Keep the listener and pass it to [`listen`].
    First(TcpListener),
    /// Another copy is running and has been asked to show its window.
    Second,
    /// The port is held by something else, so copies cannot be told apart.
    Unknown,
}

/// Finds out whether another copy is running, asking it to come forward if so.
pub fn claim() -> Claim {
    match TcpListener::bind(address()) {
        Ok(listener) => Claim::First(listener),
        Err(_) => match ask() {
            Some(()) => Claim::Second,
            None => Claim::Unknown,
        },
    }
}

fn ask() -> Option<()> {
    let mut stream = TcpStream::connect_timeout(&address(), Duration::from_secs(1)).ok()?;
    stream.set_read_timeout(Some(Duration::from_secs(2))).ok()?;
    stream.write_all(ASK).ok()?;
    let mut answer = [0u8; ANSWER.len()];
    stream.read_exact(&mut answer).ok()?;
    (answer == ANSWER).then_some(())
}

/// Answers later copies for as long as the app runs. Each one that asks sends
/// a message on `shown`, after which `wake` is called.
pub fn listen(listener: TcpListener, shown: Sender<()>, wake: impl Fn() + Send + 'static) {
    std::thread::spawn(move || {
        for mut stream in listener.incoming().flatten() {
            let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
            let mut asked = [0u8; ASK.len()];
            if stream.read_exact(&mut asked).is_ok() && asked == ASK {
                let _ = stream.write_all(ANSWER);
                if shown.send(()).is_err() {
                    return;
                }
                wake();
            }
        }
    });
}
