use bincode_next::config::{self, Configuration};
use bincode_next::{Decode, Encode};
use interprocess::TryClone;
use interprocess::local_socket::Stream;
use std::io::{Read, Write};

pub struct IpcSender {
    stream: Stream,
    config: Configuration,
}

pub struct IpcReceiver {
    stream: Stream,
    config: Configuration,
}

pub struct IpcConnection {
    stream: Stream,
    config: Configuration,
}

impl IpcConnection {
    pub fn new(stream: Stream) -> Self {
        Self {
            stream,
            config: config::standard(),
        }
    }

    /// Splits the connection into an owned sender/receiver pair that can be
    /// moved to separate threads and used concurrently.
    pub fn split(self) -> std::io::Result<(IpcSender, IpcReceiver)> {
        let read_half = self.stream.try_clone()?;
        Ok((
            IpcSender {
                stream: self.stream,
                config: self.config,
            },
            IpcReceiver {
                stream: read_half,
                config: self.config,
            },
        ))
    }
}

impl IpcSender {
    pub fn send<T: Encode>(&mut self, msg: &T) -> std::io::Result<()> {
        let data = bincode_next::encode_to_vec(msg, self.config)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
        let len = data.len() as u32;

        self.stream.write_all(&len.to_be_bytes())?;
        self.stream.write_all(&data)?;
        Ok(())
    }
}

impl IpcReceiver {
    pub fn recv<T: Decode<()>>(&mut self) -> std::io::Result<T> {
        let mut len_buf = [0u8; 4];
        self.stream.read_exact(&mut len_buf)?;

        let len = u32::from_be_bytes(len_buf) as usize;
        let mut data = vec![0u8; len];
        self.stream.read_exact(&mut data)?;

        let (msg, _bytes_read): (T, usize) = bincode_next::decode_from_slice(&data, self.config)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;

        Ok(msg)
    }
}
