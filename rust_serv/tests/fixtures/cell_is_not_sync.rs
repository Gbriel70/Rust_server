use std::{cell::Cell, sync::Arc};
fn main() {
    let counter = Arc::new(Cell::new(0));
    std::thread::spawn(move || counter.set(1));
}
