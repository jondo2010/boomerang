//! Vendored from fantasyzhjk/tokio-bincodec and LucioFranco/tokio-bincode

use std::marker::PhantomData;

use bincode::{DefaultOptions, Options};
use bytes::{Buf, BufMut, BytesMut};
use serde::{de::DeserializeOwned, Serialize};
use std::io::{self, Read};
use tokio_util::codec::{Decoder, Encoder};

/// Create a bincode based codec
#[inline]
pub fn create<T: DeserializeOwned>() -> BinCodec<T, DefaultOptions> {
    BinCodec::<T, DefaultOptions>::with_config(bincode::options())
}

/// Bincode based codec for use with `tokio-codec`
pub struct BinCodec<T, O> {
    options: O,
    _pd: PhantomData<T>,
}

impl<T: DeserializeOwned, O: Options + Copy> BinCodec<T, O> {
    /// Provides a bincode based codec from the bincode config
    #[inline]
    pub fn with_config(config: O) -> Self {
        BinCodec {
            options: config,
            _pd: PhantomData,
        }
    }
}

impl<T: DeserializeOwned, O: Options + Copy> Decoder for BinCodec<T, O> {
    type Item = T;
    type Error = bincode::Error;

    fn decode(&mut self, buf: &mut BytesMut) -> Result<Option<Self::Item>, Self::Error> {
        if !buf.is_empty() {
            let mut reader = Reader::new(&buf[..]);
            let message = self.options.deserialize_from(&mut reader)?;
            buf.advance(reader.amount());
            Ok(Some(message))
        } else {
            Ok(None)
        }
    }
}

impl<T: Serialize, O: Options + Copy> Encoder<T> for BinCodec<T, O> {
    type Error = bincode::Error;

    fn encode(&mut self, item: T, buf: &mut BytesMut) -> Result<(), Self::Error> {
        let size = self.options.serialized_size(&item)?;
        buf.reserve(size as usize);
        let message = self.options.serialize(&item)?;
        buf.put(&message[..]);
        Ok(())
    }
}

impl<T, O> std::fmt::Debug for BinCodec<T, O> {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        f.debug_struct("BinCodec").finish()
    }
}

#[derive(Debug)]
struct Reader<'buf> {
    buf: &'buf [u8],
    amount: usize,
}

impl<'buf> Reader<'buf> {
    pub fn new(buf: &'buf [u8]) -> Self {
        Reader { buf, amount: 0 }
    }

    pub fn amount(&self) -> usize {
        self.amount
    }
}

impl<'buf, 'a> Read for &'a mut Reader<'buf> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let bytes_read = self.buf.read(buf)?;
        self.amount += bytes_read;
        Ok(bytes_read)
    }
}

#[cfg(test)]
mod test {
    use super::create;
    use futures::sink::SinkExt;
    use serde::{Deserialize, Serialize};
    use std::net::SocketAddr;
    use tokio::net::{TcpListener, TcpStream};
    use tokio_stream::StreamExt;
    use tokio_util::codec::Framed;

    #[derive(Deserialize, Serialize, Debug, Clone, PartialEq)]
    enum Mock {
        One(u8),
        Two(f32),
    }

    #[tokio::test]
    async fn this_should_run() {
        let addr = SocketAddr::new("127.0.0.1".parse().unwrap(), 15151);
        let echo = TcpListener::bind(&addr).await.unwrap();
        tokio::spawn(async move {
            match echo.accept().await {
                Ok((socket, addr)) => {
                    println!("new client: {:?}", addr);
                    let mut f = Framed::new(socket, create::<Mock>());
                    while let Some(Ok(p)) = f.next().await {
                        dbg!(&p);
                        f.send(p).await.unwrap()
                    }
                }
                Err(e) => println!("couldn't get client: {:?}", e),
            }
        });

        let client = TcpStream::connect(&addr).await.unwrap();
        let mut client = Framed::new(client, create::<Mock>());
        client.send(Mock::One(1)).await.unwrap();

        let got = match client.next().await.unwrap() {
            Ok(x) => x,
            Err(e) => panic!("{e}"),
        };

        assert_eq!(got, Mock::One(1));

        client.send(Mock::Two(2.0)).await.unwrap();

        let got = match client.next().await.unwrap() {
            Ok(x) => x,
            Err(e) => panic!("{e}"),
        };

        assert_eq!(got, Mock::Two(2.0));
    }
}
