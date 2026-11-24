//! Whether a server speaks HTTP, so a browser can open it. The request asks for a version
//! no server supports, so each one turns it down in its parser: the app never sees it, no
//! route runs and nothing compiles.

use std::io::{ErrorKind, Read, Write};
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr, TcpStream};
use std::time::Duration;

const PROBE: &[u8] = b"GET / HTTP/9.9\r\nHost: localhost\r\n\r\n";
const CONNECT_WAIT: Duration = Duration::from_millis(300);
const ANSWER_WAIT: Duration = Duration::from_millis(500);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Answer {
    Web,
    /// It answered, or hung up, as no HTTP server does.
    Other,
    /// Nothing took the connection or answered in time; it may still be starting.
    Silent,
}

pub(crate) fn probe(port: u16) -> Answer {
    for address in [
        IpAddr::V4(Ipv4Addr::LOCALHOST),
        IpAddr::V6(Ipv6Addr::LOCALHOST),
    ] {
        if let Ok(mut stream) =
            TcpStream::connect_timeout(&SocketAddr::new(address, port), CONNECT_WAIT)
        {
            return answer(&mut stream);
        }
    }
    Answer::Silent
}

fn answer(stream: &mut TcpStream) -> Answer {
    let _ = stream.set_read_timeout(Some(ANSWER_WAIT));
    let _ = stream.set_write_timeout(Some(ANSWER_WAIT));
    if stream.write_all(PROBE).is_err() {
        return Answer::Other;
    }
    let mut head = [0; 5];
    let mut read = 0;
    while read < head.len() {
        match stream.read(&mut head[read..]) {
            Ok(0) => break,
            Ok(count) => read += count,
            Err(error) if matches!(error.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut) => {
                if read == 0 {
                    return Answer::Silent;
                }
                break;
            }
            Err(_) => break,
        }
    }
    kind(&head[..read])
}

/// Python's servers turn the version down with their error page alone, without a status line.
fn kind(head: &[u8]) -> Answer {
    let lower = head.to_ascii_lowercase();
    if head.starts_with(b"HTTP/") || lower.starts_with(b"<!doc") || lower.starts_with(b"<html") {
        Answer::Web
    } else {
        Answer::Other
    }
}

#[cfg(test)]
mod tests {
    use super::{Answer, kind, probe};
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::thread;

    fn serve(reply: &'static [u8]) -> u16 {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        thread::spawn(move || {
            if let Ok((mut stream, _)) = listener.accept() {
                let mut request = [0; 64];
                let _ = stream.read(&mut request);
                let _ = stream.write_all(reply);
            }
        });
        port
    }

    #[test]
    fn an_http_answer_or_an_error_page_is_a_web_server() {
        assert_eq!(kind(b"HTTP/"), Answer::Web);
        assert_eq!(kind(b"<!DOC"), Answer::Web);
        assert_eq!(kind(b"<html"), Answer::Web);
        assert_eq!(kind(b"-ERR "), Answer::Other);
        assert_eq!(kind(b""), Answer::Other);
    }

    #[test]
    fn a_server_is_told_by_how_it_answers_the_probe() {
        assert_eq!(
            probe(serve(b"HTTP/1.1 400 Bad Request\r\n\r\n")),
            Answer::Web
        );
        assert_eq!(probe(serve(b"-ERR unknown command\r\n")), Answer::Other);
        assert_eq!(probe(serve(b"")), Answer::Other);
        let closed = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = closed.local_addr().unwrap().port();
        drop(closed);
        assert_eq!(probe(port), Answer::Silent);
    }
}
