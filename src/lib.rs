use either::Either;
use futures::{
    stream::{self, IterOk, Once, Stream},
    Async, Poll,
};
use std::{
    io,
    iter::Cloned,
    net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr, SocketAddrV4, SocketAddrV6, ToSocketAddrs},
    slice::Iter,
    vec,
};

pub use error::Error;
use error::Result;

/// A trait for objects which can be converted or resolved to one or more `SocketAddr` values,
/// which are going to be connected as the the proxy server.
///
/// This trait is similar to `std::net::ToSocketAddrs` but allows asynchronous name resolution.
pub trait ToProxyAddrs {
    type Output: Stream<Item = SocketAddr, Error = Error>;

    fn to_proxy_addrs(&self) -> Self::Output;
}

macro_rules! trivial_impl_to_proxy_addrs {
    ($t: ty) => {
        impl ToProxyAddrs for $t {
            type Output = Once<SocketAddr, Error>;

            fn to_proxy_addrs(&self) -> Self::Output {
                stream::once(Ok(SocketAddr::from(*self)))
            }
        }
    };
}

trivial_impl_to_proxy_addrs!(SocketAddr);
trivial_impl_to_proxy_addrs!((IpAddr, u16));
trivial_impl_to_proxy_addrs!((Ipv4Addr, u16));
trivial_impl_to_proxy_addrs!((Ipv6Addr, u16));
trivial_impl_to_proxy_addrs!(SocketAddrV4);
trivial_impl_to_proxy_addrs!(SocketAddrV6);

impl<'a> ToProxyAddrs for &'a [SocketAddr] {
    type Output = IterOk<Cloned<Iter<'a, SocketAddr>>, Error>;

    fn to_proxy_addrs(&self) -> Self::Output {
        stream::iter_ok(self.iter().cloned())
    }
}

impl ToProxyAddrs for str {
    type Output = ProxyAddrsStream;

    fn to_proxy_addrs(&self) -> Self::Output {
        ProxyAddrsStream(Some(self.to_socket_addrs()))
    }
}

impl<'a> ToProxyAddrs for (&'a str, u16) {
    type Output = ProxyAddrsStream;

    fn to_proxy_addrs(&self) -> Self::Output {
        ProxyAddrsStream(Some(self.to_socket_addrs()))
    }
}

impl<'a, T: ToProxyAddrs + ?Sized> ToProxyAddrs for &'a T {
    type Output = T::Output;

    fn to_proxy_addrs(&self) -> Self::Output {
        (**self).to_proxy_addrs()
    }
}

pub struct ProxyAddrsStream(Option<io::Result<vec::IntoIter<SocketAddr>>>);

impl Stream for ProxyAddrsStream {
    type Item = SocketAddr;
    type Error = Error;

    fn poll(&mut self) -> Poll<Option<SocketAddr>, Self::Error> {
        if let Some(res) = &mut self.0 {
            if let Ok(iter) = res {
                return Ok(Async::Ready(iter.next()));
            }
            // res is err
            let _ = self.0.take().unwrap()?;
        }
        unreachable!()
    }
}

/// A SOCKS connection target.
#[derive(Debug, PartialEq, Eq)]
pub enum TargetAddr {
    /// Connect to an IP address.
    Ip(SocketAddr),

    /// Connect to a fully-qualified domain name.
    ///
    /// The domain name will be passed along to the proxy server and DNS lookup will happen there.
    Domain(String, u16),
}

impl TargetAddr {
    /// Creates owned `TargetAddr` by cloning. It is usually used to eliminate the lifetime bound.
    pub fn to_owned(&self) -> TargetAddr {
        match self {
            TargetAddr::Ip(addr) => TargetAddr::Ip(*addr),
            TargetAddr::Domain(domain, port) => {
                TargetAddr::Domain(String::from(domain.clone()).into(), *port)
            }
        }
    }
}

impl ToSocketAddrs for TargetAddr {
    type Iter = Either<std::option::IntoIter<SocketAddr>, std::vec::IntoIter<SocketAddr>>;

    fn to_socket_addrs(&self) -> io::Result<Self::Iter> {
        Ok(match self {
            TargetAddr::Ip(addr) => Either::Left(addr.to_socket_addrs()?),
            TargetAddr::Domain(domain, port) => {
                Either::Right((&**domain, *port).to_socket_addrs()?)
            }
        })
    }
}

/// A trait for objects that can be converted to `TargetAddr`.
pub trait IntoTargetAddr {
    /// Converts the value of self to a `TargetAddr`.
    fn into_target_addr(self) -> Result<TargetAddr>;
}

macro_rules! trivial_impl_into_target_addr {
    ($t: ty) => {
        impl IntoTargetAddr for $t {
            fn into_target_addr(self) -> Result<TargetAddr> {
                Ok(TargetAddr::Ip(SocketAddr::from(self)))
            }
        }
    };
}

trivial_impl_into_target_addr!(SocketAddr);
trivial_impl_into_target_addr!((IpAddr, u16));
trivial_impl_into_target_addr!((Ipv4Addr, u16));
trivial_impl_into_target_addr!((Ipv6Addr, u16));
trivial_impl_into_target_addr!(SocketAddrV4);
trivial_impl_into_target_addr!(SocketAddrV6);

impl IntoTargetAddr for (&str, u16) {
    fn into_target_addr(self) -> Result<TargetAddr> {
        // Try IP address first
        if let Ok(addr) = self.0.parse::<IpAddr>() {
            return (addr, self.1).into_target_addr();
        }

        // Treat as domain name
        let len = self.0.as_bytes().len();
        if len > 255 {
            return Err(Error::InvalidTargetAddress("overlong domain"));
        }
        // TODO: Should we validate the domain format here?

        Ok(TargetAddr::Domain(self.0.into(), self.1))
    }
}

impl IntoTargetAddr for &str {
    fn into_target_addr(self) -> Result<TargetAddr> {
        // Try IP address first
        if let Ok(addr) = self.parse::<SocketAddr>() {
            return addr.into_target_addr();
        }

        let mut parts_iter = self.rsplitn(2, ':');
        let port: u16 = parts_iter
            .next()
            .and_then(|port_str| port_str.parse().ok())
            .ok_or(Error::InvalidTargetAddress("invalid address format"))?;
        let domain = parts_iter
            .next()
            .ok_or(Error::InvalidTargetAddress("invalid address format"))?;
        (domain, port).into_target_addr()
    }
}

impl IntoTargetAddr for (String, u16) {
    fn into_target_addr(self) -> Result<TargetAddr> {
        let addr = (self.0.as_str(), self.1).into_target_addr()?;
        if let TargetAddr::Ip(addr) = addr {
            Ok(TargetAddr::Ip(addr))
        } else {
            Ok(TargetAddr::Domain(self.0.into(), self.1))
        }
    }
}

impl<T> IntoTargetAddr for &T
where
    T: IntoTargetAddr + Copy,
{
    fn into_target_addr(self) -> Result<TargetAddr> {
        (*self).into_target_addr()
    }
}

/// Authentication methods
#[derive(Debug)]
enum Authentication {
    Password {
        username: String,
        password: String,
    },
    None,
}

impl Authentication {
    fn id(&self) -> u8 {
        match self {
            Authentication::Password { .. } => 0x02,
            Authentication::None => 0x00,
        }
    }
}

mod error;
pub mod tcp;

#[cfg(test)]
mod tests {
    use super::*;

    fn to_proxy_addrs<T: ToProxyAddrs>(t: T) -> Result<Vec<SocketAddr>> {
        t.to_proxy_addrs().wait().collect()
    }

    #[test]
    fn converts_socket_addr_to_proxy_addrs() -> Result<()> {
        let addr = SocketAddr::from(([1, 1, 1, 1], 443));
        let res = to_proxy_addrs(addr)?;
        assert_eq!(&res[..], &[addr]);
        Ok(())
    }

    #[test]
    fn converts_socket_addr_ref_to_proxy_addrs() -> Result<()> {
        let addr = SocketAddr::from(([1, 1, 1, 1], 443));
        let res = to_proxy_addrs(&addr)?;
        assert_eq!(&res[..], &[addr]);
        Ok(())
    }

    #[test]
    fn converts_socket_addrs_to_proxy_addrs() -> Result<()> {
        let addrs = [
            SocketAddr::from(([1, 1, 1, 1], 443)),
            SocketAddr::from(([8, 8, 8, 8], 53)),
        ];
        let res = to_proxy_addrs(&addrs[..])?;
        assert_eq!(&res[..], &addrs);
        Ok(())
    }

    fn into_target_addr<T>(t: T) -> Result<TargetAddr>
    where
        T: IntoTargetAddr,
    {
        t.into_target_addr()
    }

    #[test]
    fn converts_socket_addr_to_target_addr() -> Result<()> {
        let addr = SocketAddr::from(([1, 1, 1, 1], 443));
        let res = into_target_addr(addr)?;
        assert_eq!(TargetAddr::Ip(addr), res);
        Ok(())
    }

    #[test]
    fn converts_socket_addr_ref_to_target_addr() -> Result<()> {
        let addr = SocketAddr::from(([1, 1, 1, 1], 443));
        let res = into_target_addr(&addr)?;
        assert_eq!(TargetAddr::Ip(addr), res);
        Ok(())
    }

    #[test]
    fn converts_socket_addr_str_to_target_addr() -> Result<()> {
        let addr = SocketAddr::from(([1, 1, 1, 1], 443));
        let ip_str = format!("{}", addr);
        let res = into_target_addr(ip_str.as_str())?;
        assert_eq!(TargetAddr::Ip(addr), res);
        Ok(())
    }

    #[test]
    fn converts_ip_str_and_port_target_addr() -> Result<()> {
        let addr = SocketAddr::from(([1, 1, 1, 1], 443));
        let ip_str = format!("{}", addr.ip());
        let res = into_target_addr((ip_str.as_str(), addr.port()))?;
        assert_eq!(TargetAddr::Ip(addr), res);
        Ok(())
    }

    #[test]
    fn converts_domain_to_target_addr() -> Result<()> {
        let domain = "www.example.com:80";
        let res = into_target_addr(domain)?;
        assert_eq!(
            TargetAddr::Domain("www.example.com".to_string(), 80),
            res
        );
        Ok(())
    }

    #[test]
    fn converts_domain_and_port_to_target_addr() -> Result<()> {
        let domain = "www.example.com";
        let res = into_target_addr((domain, 80))?;
        assert_eq!(
            TargetAddr::Domain("www.example.com".to_string(), 80),
            res
        );
        Ok(())
    }

    #[test]
    fn overlong_domain_to_target_addr_should_fail() {
        let domain = format!("www.{:a<1$}.com:80", 'a', 300);
        assert!(into_target_addr(domain.as_str()).is_err());
        let domain = format!("www.{:a<1$}.com", 'a', 300);
        assert!(into_target_addr((domain.as_str(), 80)).is_err());
    }

    #[test]
    fn addr_with_invalid_port_to_target_addr_should_fail() {
        let addr = "[ffff::1]:65536";
        assert!(into_target_addr(addr).is_err());
        let addr = "www.example.com:65536";
        assert!(into_target_addr(addr).is_err());
    }
}
