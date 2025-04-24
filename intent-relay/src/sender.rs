use crate::intent::AuthedIntent;
use log::error;
use tokio::io::AsyncWriteExt;
use tokio::net::TcpStream;

#[async_trait::async_trait]
pub trait Sender<T> {
    async fn send(&mut self, message: T);
}

pub struct TcpSender {
    conn: TcpStream,
}

impl TcpSender {
    pub async fn new(addr: String) -> Option<Self> {
        if let Ok(stream) = TcpStream::connect(&addr).await {
            Some(Self { conn: stream })
        } else {
            None
        }
    }
}

#[async_trait::async_trait]
impl Sender<AuthedIntent> for TcpSender {
    async fn send(&mut self, message: AuthedIntent) {
        if let Err(e) = self.conn.write_all(message.encode().as_slice()).await {
            error!("Failed to send message: {}", e);
        }
    }
}
