#![feature(trivial_bounds)]
#![allow(non_upper_case_globals)]
#![allow(non_camel_case_types)]
#![allow(non_snake_case)]

use const_default::ConstDefault;

include!(concat!(env!("OUT_DIR"), "/wrapper.rs"));
