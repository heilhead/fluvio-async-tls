use async_std::net::TcpStream;
use async_std::task;
use fluvio_async_tls::TlsConnector;
use futures_util::io::{AsyncReadExt, AsyncWriteExt};

#[test]
fn fetch_mozilla() -> std::io::Result<()> {
    task::block_on(async {
        let connector = TlsConnector::default();

        let stream = TcpStream::connect("mozilla.org:443").await?;
        let mut stream = connector.connect("mozilla.org", stream).await?;

        stream
            .write_all(
                concat!(
                    "GET / HTTP/1.1\r\n",
                    "Host: mozilla.org\r\n",
                    "Connection: close\r\n",
                    "\r\n"
                )
                .as_bytes(),
            )
            .await?;
        let mut res = vec![];
        stream.read_to_end(&mut res).await?;

        let data = String::from_utf8_lossy(&res);
        println!("{}", &data);

        assert!(data.starts_with("HTTP/1.1 "));

        let data = data.trim_end();
        assert!(data.ends_with("</html>") || data.ends_with("</HTML>"));
        Ok(())
    })
}
