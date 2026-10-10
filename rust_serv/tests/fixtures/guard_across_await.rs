// Models tokio::spawn's Future + Send + 'static bound using only std, so rustc
// can compile this fixture without wiring external Cargo dependencies.
use std::{future::Future,sync::{Arc,Mutex}};
fn spawn(_future:impl Future<Output=()>+Send+'static){}
fn main(){
    let state=Arc::new(Mutex::new(0));
    spawn(async move{
        let guard=state.lock().unwrap();
        std::future::pending::<()>().await;
        drop(guard);
    });
}
