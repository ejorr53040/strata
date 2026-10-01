mod common;

fn it() {
    common::setup();
}

use common::setup;

fn it2() {
    setup();
}
