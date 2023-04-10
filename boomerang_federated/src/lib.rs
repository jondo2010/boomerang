#[cfg(test)]
mod tests {
    use canary::providers::Addr;
    use tokio_test::block_on;

    #[test]
    fn test() {
        let f = async {
            let addr = "unix@localhost".parse::<Addr>().unwrap();
            addr.bind().await.unwrap();
        };

        block_on(f);
    }
}
