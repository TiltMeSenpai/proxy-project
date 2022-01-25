use std::io::IoSlice;
use std::io::ErrorKind;

use futures::{AsyncRead, AsyncWrite};
use futures::{AsyncReadExt, AsyncWriteExt};

pub trait Storable {
    type Header: Sized;

    fn pack(&self) -> (Self::Header, &[IoSlice]);
    fn pack_size(header: &Self::Header) -> usize;
    fn unpack(hdr: &Self::Header, pack: &[u8]) -> Self;
}

pub async fn read_stored<F: AsyncRead + Unpin, S: Storable>(mut f: F, header: &S::Header) -> std::io::Result<S> {
    let size = S::pack_size(header);
    let mut buf: Vec<u8> = Vec::with_capacity(size);
    let buf_slice: &mut [u8] = buf.as_mut_slice();
    match f.read_exact(buf_slice).await {
        Ok(()) => Ok(S::unpack(header, buf_slice)),
        Err(e) => Err(e),
    }
}

pub async fn write_stored<F: AsyncWrite + Unpin, S: Storable>(mut f: F, store: S) -> std::io::Result<S::Header> {
    let (header, pack) = store.pack();
    let len: usize = pack.iter().map(|slice| slice.len()).sum();
    match f.write_vectored(pack).await {
        Ok(bytes_written) => {
            if bytes_written < len {
                Err(std::io::Error::new(ErrorKind::UnexpectedEof, format!("Not enough bytes written, wanted {} got {}", len, bytes_written)))
            } else {
                Ok(header)
            }
        },
        Err(e) => Err(e),
    }
}