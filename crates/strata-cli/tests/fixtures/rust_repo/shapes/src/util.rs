use crate::Circle;

pub fn square(x: f64) -> f64 {
    x * x
}

pub fn make() -> Circle {
    let c = Circle::new(1.0);
    println!("made");
    c
}

pub(crate) fn hidden() {}

#[cfg(test)]
mod inline_tests {
    use super::*;

    const K: f64 = 1.0;

    fn t() {
        square(K);
    }
}

pub(crate) mod vis_mod {}

impl crate::Circle {
    pub fn extra(&self) -> f64 {
        1.0
    }
}
