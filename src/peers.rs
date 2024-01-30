use crate::download::BlockRequest;
use crate::logger::{log, LogLevel};
use crate::torrent::Torrent;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

#[derive(Debug)]
pub struct Peer {
    pub peer_id: Option<String>,
    pub bitfield: Option<Vec<u8>>,
    pub status: PeerStatus,
    socket: TcpStream,
}

#[derive(Debug)]
pub enum PeerStatus {
    NotConnected,
    WaitingForInterestedMsg,
    ReadyToSendData
}

impl Peer {
    // just for test 9 (with hanshake only)
    pub async fn new(addr: &str) -> anyhow::Result<Peer> {
        Ok(Peer {
            peer_id: None,
            socket: TcpStream::connect(addr).await?,
            bitfield: None,
            status: PeerStatus::NotConnected
        })
    }
    pub async fn connect(&mut self, torrent: &Torrent) -> anyhow::Result<()> {
        match self.status {
            PeerStatus::ReadyToSendData => Ok(()),
            PeerStatus::WaitingForInterestedMsg => {
                self.send_interested_msg().await?;
                Ok(())
            },
            PeerStatus::NotConnected => {
                log!(LogLevel::Debug, "Connecting to peer: {}", self.socket.peer_addr()?);
                let peer_id = Peer::handshake(&torrent, &mut self.socket).await?;
                let mut msg_len = [0; 4];
                self.socket.read_exact(&mut msg_len).await?;
                let msg_len = u32::from_be_bytes(msg_len);
                let mut data = vec![0; msg_len as usize];
                self.socket.read_exact(&mut data).await?;
                assert_eq!(data[0], 5);
                log!(LogLevel::Debug, "Got peer bitfield");
                self.peer_id = Some(peer_id);
                self.bitfield = Some(data[1..].to_vec());
                self.status = PeerStatus::WaitingForInterestedMsg;
                self.send_interested_msg().await?;
                self.status = PeerStatus::ReadyToSendData;
                Ok(())
            }
        }
    }

    pub async fn fetch(&mut self, req: &BlockRequest) -> anyhow::Result<Vec<u8>> {
        self.socket.write_all(&13u32.to_be_bytes()).await?;
        self.socket.write_all(&6u8.to_be_bytes()).await?;
        self.socket.write_all(&req.piece_i.to_be_bytes()).await?;
        self.socket.write_all(&req.begin.to_be_bytes()).await?;
        self.socket.write_all(&req.length.to_be_bytes()).await?;
        log!(LogLevel::Debug, "Requested {:?}", req);
        let mut data = [0; 13];
        self.socket.read_exact(&mut data).await?;
        assert_eq!(data[4], 7); // piece message
        let mut buf = vec![0; req.length as usize];
        self.socket.read_exact(&mut buf).await?;
        log!(LogLevel::Debug, "Readed block from socket");
        Ok(buf)
    }

    async fn send_interested_msg(&mut self) -> anyhow::Result<()> {
        self.socket.write_all(&1u32.to_be_bytes()).await?;
        self.socket.write_all(&2u8.to_be_bytes()).await?;
        log!(LogLevel::Debug, "Sended interested msg");
        let mut data = [0; 5];
        self.socket.read_exact(&mut data).await?;
        log!(LogLevel::Debug, "Received unchoke msg");
        assert_eq!(data[4], 1); // unchoke message
        Ok(())
    }

    async fn handshake(torrent: &Torrent, stream: &mut TcpStream) -> anyhow::Result<String> {
        let mut msg = Vec::new();
        msg.push(b"\x13"[0]); // 0x13 = 19
        msg.extend_from_slice(b"BitTorrent protocol");
        msg.extend_from_slice(&[0; 8]);
        msg.extend_from_slice(&torrent.info_hash);
        msg.extend_from_slice(b"00112233445566770099");
        stream.write_all(&msg).await?;
        log!(LogLevel::Debug, "Sended handskake");
        let mut response = [0; 68];
        stream.read_exact(&mut response).await?;
        log!(LogLevel::Debug, "Received answer handskake");
        let peer_id = &response[response.len() - 20..response.len()];
        Ok(hex::encode(peer_id))
    }
}
