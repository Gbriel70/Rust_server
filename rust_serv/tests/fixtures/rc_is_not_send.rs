use std::rc::Rc;
fn main() {
    let value = Rc::new(String::from("shared only inside one thread"));
    std::thread::spawn(move || drop(value));
}
