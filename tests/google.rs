use async_std::net::TcpStream;
use async_std::task;
use fluvio_async_tls::TlsConnector;
use futures_util::io::{AsyncReadExt, AsyncWriteExt};

#[test]
fn fetch_google() -> std::io::Result<()> {
    task::block_on(async {
        let connector = TlsConnector::default();

        let stream = TcpStream::connect("google.com:443").await?;
        let mut stream = connector.connect("google.com", stream).await?;

        stream.write_all(b"GET / HTTP/1.0\r\n\r\n").await?;
        let mut res = vec![];

        // Google might answer with a close_notify or not.
        match stream.read_to_end(&mut res).await {
            Ok(_) => (),
            Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => (),
            Err(e) => return Err(e),
        }

        let data = String::from_utf8_lossy(&res);

        assert!(data.starts_with("HTTP/1.0 "));

        let data = data.trim_end();
        assert!(data.ends_with("</html>") || data.ends_with("</HTML>"));
        Ok(())
    })
}
