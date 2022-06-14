use super::Stream;
use futures_executor::block_on;
use futures_io::{AsyncRead, AsyncWrite};
use futures_util::io::{AsyncReadExt, AsyncWriteExt};
use futures_util::task::{noop_waker_ref, Context};
use futures_util::{future, ready};
use rustls::{
    Certificate, ClientConfig, ClientConnection, Connection, PrivateKey, RootCertStore,
    ServerConfig, ServerConnection, ServerName,
};
use rustls_pemfile::{certs, rsa_private_keys};
use std::pin::Pin;
use std::sync::Arc;
use std::task::Poll;
use std::{
    convert::TryFrom,
    io::{self, BufReader, Cursor, Read, Write},
};

struct Good<'a>(&'a mut Connection);

impl<'a> AsyncRead for Good<'a> {
    fn poll_read(
        mut self: Pin<&mut Self>,
        _cx: &mut Context<'_>,
        mut buf: &mut [u8],
    ) -> Poll<io::Result<usize>> {
        Poll::Ready(self.0.write_tls(buf.by_ref()))
    }
}

impl<'a> AsyncWrite for Good<'a> {
    fn poll_write(
        mut self: Pin<&mut Self>,
        _cx: &mut Context<'_>,
        mut buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        let len = self.0.read_tls(buf.by_ref())?;
        self.0
            .process_new_packets()
            .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))?;
        Poll::Ready(Ok(len))
    }

    fn poll_flush(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Poll::Ready(Ok(()))
    }

    fn poll_close(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Poll::Ready(Ok(()))
    }
}

struct Bad(bool);

impl AsyncRead for Bad {
    fn poll_read(
        self: Pin<&mut Self>,
        _cx: &mut Context<'_>,
        _: &mut [u8],
    ) -> Poll<io::Result<usize>> {
        Poll::Ready(Ok(0))
    }
}

impl AsyncWrite for Bad {
    fn poll_write(
        self: Pin<&mut Self>,
        _cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        if self.0 {
            Poll::Pending
        } else {
            Poll::Ready(Ok(buf.len()))
        }
    }

    fn poll_flush(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Poll::Ready(Ok(()))
    }

    fn poll_close(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Poll::Ready(Ok(()))
    }
}

#[test]
fn stream_good() -> io::Result<()> {
    const FILE: &'static [u8] = include_bytes!("../../README.md");

    let fut = async {
        let (mut server, mut client) = make_pair();
        future::poll_fn(|cx| do_handshake(&mut client, &mut server, cx)).await?;
        io::copy(&mut Cursor::new(FILE), &mut server.writer())?;

        {
            let mut good = Good(&mut server);
            let mut stream = Stream::new(&mut good, &mut client);

            let mut buf = Vec::new();
            stream.read_to_end(&mut buf).await?;
            assert_eq!(buf, FILE);
            stream.write_all(b"Hello World!").await?;
        }

        let mut buf = String::new();
        server.reader().read_to_string(&mut buf)?;
        assert_eq!(buf, "Hello World!");

        Ok(()) as io::Result<()>
    };

    block_on(fut)
}

#[test]
fn stream_bad() -> io::Result<()> {
    let fut = async {
        let (mut server, mut client) = make_pair();
        future::poll_fn(|cx| do_handshake(&mut client, &mut server, cx)).await?;
        client.set_buffer_limit(Some(1024));

        let mut bad = Bad(true);
        let mut stream = Stream::new(&mut bad, &mut client);
        assert_eq!(
            future::poll_fn(|cx| stream.as_mut_pin().poll_write(cx, &[0x42; 8])).await?,
            8
        );
        assert_eq!(
            future::poll_fn(|cx| stream.as_mut_pin().poll_write(cx, &[0x42; 8])).await?,
            8
        );
        let r = future::poll_fn(|cx| stream.as_mut_pin().poll_write(cx, &[0x00; 1024])).await?; // fill buffer
        assert!(r < 1024);

        let mut cx = Context::from_waker(noop_waker_ref());
        assert!(stream
            .as_mut_pin()
            .poll_write(&mut cx, &[0x01])
            .is_pending());

        Ok(()) as io::Result<()>
    };

    block_on(fut)
}

#[test]
fn stream_handshake() -> io::Result<()> {
    let fut = async {
        let (mut server, mut client) = make_pair();

        {
            let mut good = Good(&mut server);
            let mut stream = Stream::new(&mut good, &mut client);
            let (r, w) = future::poll_fn(|cx| stream.complete_io(cx)).await?;

            assert!(r > 0);
            assert!(w > 0);

            future::poll_fn(|cx| stream.complete_io(cx)).await?; // finish server handshake
        }

        assert!(!server.is_handshaking());
        assert!(!client.is_handshaking());

        Ok(()) as io::Result<()>
    };

    block_on(fut)
}

#[test]
fn stream_handshake_eof() -> io::Result<()> {
    let fut = async {
        let (_, mut client) = make_pair();

        let mut bad = Bad(false);
        let mut stream = Stream::new(&mut bad, &mut client);

        let mut cx = Context::from_waker(noop_waker_ref());
        let r = stream.complete_io(&mut cx);
        assert_eq!(
            r.map_err(|err| err.kind()),
            Poll::Ready(Err(io::ErrorKind::UnexpectedEof))
        );

        Ok(()) as io::Result<()>
    };

    block_on(fut)
}

#[test]
fn stream_eof() -> io::Result<()> {
    let fut = async {
        let (mut server, mut client) = make_pair();
        future::poll_fn(|cx| do_handshake(&mut client, &mut server, cx)).await?;

        let mut good = Good(&mut server);
        let mut stream = Stream::new(&mut good, &mut client).set_eof(true);

        let mut buf = Vec::new();
        stream.read_to_end(&mut buf).await?;
        assert_eq!(buf.len(), 0);

        Ok(()) as io::Result<()>
    };

    block_on(fut)
}

fn make_pair() -> (Connection, Connection) {
    const CERT: &str = include_str!("../../tests/end.cert");
    const CHAIN: &str = include_str!("../../tests/end.chain");
    const RSA: &str = include_str!("../../tests/end.rsa");

    let cert = certs(&mut BufReader::new(Cursor::new(CERT)))
        .map(|certs| certs.into_iter().map(Certificate).collect())
        .unwrap();

    let mut keys = rsa_private_keys(&mut BufReader::new(Cursor::new(RSA))).unwrap();
    let sconfig = ServerConfig::builder()
        .with_safe_defaults()
        .with_no_client_auth()
        .with_single_cert(cert, keys.pop().map(PrivateKey).unwrap())
        .unwrap();

    let server = ServerConnection::new(Arc::new(sconfig)).unwrap();

    let domain = ServerName::try_from("localhost").unwrap();

    let certs = certs(&mut BufReader::new(Cursor::new(CHAIN)))
        .unwrap()
        .into_iter()
        .map(Certificate)
        .collect::<Vec<_>>();

    let mut root_store = RootCertStore::empty();

    for cert in &certs {
        root_store.add(cert).unwrap();
    }

    let cconfig = ClientConfig::builder()
        .with_safe_default_cipher_suites()
        .with_safe_default_kx_groups()
        .with_safe_default_protocol_versions()
        .unwrap()
        .with_root_certificates(root_store)
        .with_no_client_auth();

    let client = ClientConnection::new(Arc::new(cconfig), domain).unwrap();

    (server.into(), client.into())
}

fn do_handshake(
    client: &mut Connection,
    server: &mut Connection,
    cx: &mut Context<'_>,
) -> Poll<io::Result<()>> {
    let mut good = Good(server);
    let mut stream = Stream::new(&mut good, client);

    if stream.session.is_handshaking() {
        ready!(stream.complete_io(cx))?;
    }

    if stream.session.wants_write() {
        ready!(stream.complete_io(cx))?;
    }

    Poll::Ready(Ok(()))
}
