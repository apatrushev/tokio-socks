mod common;

use common::{test_bind, test_connect, ECHO_SERVER_ADDR, PROXY_ADDR};
use tokio_socks::{Result, Socks5Listener, Socks5Stream};

#[test]
fn connect() -> Result<()> {
    let conn =
        Socks5Stream::connect_with_password(PROXY_ADDR, ECHO_SERVER_ADDR, "mylogin", "mypassword")?;
    test_connect(conn)
}

#[test]
fn bind() -> Result<()> {
    let bind =
        Socks5Listener::bind_with_password(PROXY_ADDR, ECHO_SERVER_ADDR, "mylogin", "mypassword")?;
    test_bind(bind)
}
