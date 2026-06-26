pub mod types;
pub mod net;

pub fn orchestrate() {
    let mut flag = true;
    flag = !flag;
    types::process();
    net::connect();
}
