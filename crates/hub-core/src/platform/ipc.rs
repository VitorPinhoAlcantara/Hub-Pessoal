use std::io;

use tokio::io::{AsyncRead, AsyncWrite};

pub trait IpcStream: AsyncRead + AsyncWrite + Unpin + Send {}

impl<T: AsyncRead + AsyncWrite + Unpin + Send> IpcStream for T {}

pub fn ipc_path(name: &str) -> String {
    #[cfg(windows)]
    {
        format!(r"\\.\pipe\{name}")
    }
    #[cfg(not(windows))]
    {
        let base = std::env::var_os("XDG_RUNTIME_DIR")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(std::env::temp_dir);
        base.join(format!("{name}.sock"))
            .to_string_lossy()
            .into_owned()
    }
}

pub async fn connect(path: &str) -> io::Result<Box<dyn IpcStream>> {
    #[cfg(windows)]
    {
        let client = tokio::net::windows::named_pipe::ClientOptions::new().open(path)?;
        Ok(Box::new(client))
    }
    #[cfg(not(windows))]
    {
        let stream = tokio::net::UnixStream::connect(path).await?;
        Ok(Box::new(stream))
    }
}

pub struct IpcListener {
    path: String,
    #[cfg(windows)]
    next: Option<tokio::net::windows::named_pipe::NamedPipeServer>,
    #[cfg(not(windows))]
    inner: tokio::net::UnixListener,
}

impl IpcListener {
    pub fn bind(path: &str) -> io::Result<Self> {
        #[cfg(windows)]
        {
            let server = tokio::net::windows::named_pipe::ServerOptions::new()
                .first_pipe_instance(true)
                .create(path)?;
            Ok(Self {
                path: path.to_string(),
                next: Some(server),
            })
        }
        #[cfg(not(windows))]
        {
            let _ = std::fs::remove_file(path);
            let inner = tokio::net::UnixListener::bind(path)?;
            Ok(Self {
                path: path.to_string(),
                inner,
            })
        }
    }

    pub async fn accept(&mut self) -> io::Result<Box<dyn IpcStream>> {
        #[cfg(windows)]
        {
            let server = match self.next.take() {
                Some(server) => server,
                None => tokio::net::windows::named_pipe::ServerOptions::new().create(&self.path)?,
            };
            server.connect().await?;
            self.next =
                Some(tokio::net::windows::named_pipe::ServerOptions::new().create(&self.path)?);
            Ok(Box::new(server))
        }
        #[cfg(not(windows))]
        {
            let (stream, _) = self.inner.accept().await?;
            Ok(Box::new(stream))
        }
    }

    pub fn path(&self) -> &str {
        &self.path
    }
}

#[cfg(test)]
mod tests {
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

    use super::*;

    #[tokio::test(flavor = "multi_thread")]
    async fn client_and_server_exchange_lines() {
        let name = format!("hub-teste-{}", std::process::id());
        let path = ipc_path(&name);
        let mut listener = IpcListener::bind(&path).unwrap();
        let server = tokio::spawn(async move {
            let stream = listener.accept().await.unwrap();
            let mut reader = BufReader::new(stream);
            let mut line = String::new();
            reader.read_line(&mut line).await.unwrap();
            reader
                .get_mut()
                .write_all(format!("eco {line}").as_bytes())
                .await
                .unwrap();
        });
        let mut client = connect(&path).await.unwrap();
        client.write_all(b"oi\n").await.unwrap();
        let mut reader = BufReader::new(client);
        let mut answer = String::new();
        reader.read_line(&mut answer).await.unwrap();
        assert_eq!(answer, "eco oi\n");
        server.await.unwrap();
    }

    #[test]
    fn path_contains_name() {
        assert!(ipc_path("hub-mpv").contains("hub-mpv"));
    }
}
