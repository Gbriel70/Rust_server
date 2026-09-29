#[cfg(test)] 
mod tests{
    #[test]
    fn run_server(){
        let result = rust_serv::server::run();
        assert!(result.is_ok());
    }
}
