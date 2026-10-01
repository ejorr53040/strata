mod common;

fn it() {
    common::setup();
}

use common::setup;

fn it2() {
    setup();
}

#[path = "../src/helper.rs"]
mod helper_alias;

fn it3() {
    helper_alias::go();
}
