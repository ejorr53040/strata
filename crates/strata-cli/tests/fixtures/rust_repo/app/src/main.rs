use shapes::util::square;
use shapes::{Circle, Shape};

fn main() {
    let c = Circle::new(2.0);
    c.area();
    square(3.0);
    std::process::exit(0);
}

use shapes::util;

fn shadow() {
    let util = Circle::new(1.0);
    util.area();
}
