pub struct Config {
    pub enabled: bool,
    pub verbose: bool,
}

pub trait Runnable {
    fn run(&self);
}

impl Runnable for Config {
    fn run(&self) {}
}

pub enum State {
    Ready,
    Running(bool),
    Done,
}

pub fn process() {
    let mut state = true;
    state = !state;
}

#[cfg(test)]
mod tests {
    #[test]
    fn test_nothing() {}
}
