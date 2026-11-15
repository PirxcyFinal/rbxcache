//! Keeping to one window. Two copies would both follow the cache and write
//! over each other's replacements, so a second start hands over to the first.
//!
//! The first copy listens on a loopback port. A later copy that finds the
//! port taken asks whoever holds it to show its window, and exits if the
//! answer is the one this app gives.

use std::io::{Read, Write};
use std::net::{Ipv4Addr, SocketAddr, TcpListener, TcpStream};
use std::sync::mpsc::Sender;
use std::time::Duration;

/// Chosen at random from the range no service is registered in.
const PORT: u16 = 47_163;
const ASK: &[u8] = b"rbxcache show\n";
const ANSWER: &[u8] = b"rbxcache shown\n";

fn address() -> SocketAddr {
    SocketAddr::from((Ipv4Addr::LOCALHOST, PORT))
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
