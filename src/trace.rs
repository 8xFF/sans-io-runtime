use std::fmt::Debug;

pub trait ErrorDebugger {
    fn print_err(&self, msg: &str);
}

pub trait ErrorDebugger2 {
    fn print_err2(&self, msg: &str);
}

impl<T, E: Debug> ErrorDebugger for Result<T, E> {
    fn print_err(&self, msg: &str) {
        match self {
            Ok(_) => {}
            Err(e) => {
                log::error!("Error, {msg}: {:?}", e);
            }
        }
    }
}

impl<T, E> ErrorDebugger2 for Result<T, E> {
    fn print_err2(&self, msg: &str) {
        match self {
            Ok(_) => {}
            Err(e) => {
                log::error!("Error, {msg}");
            }
        }
    }
}

pub trait OptionDebugger {
    fn print_none(&self, msg: &str);
}

impl<T: Debug> OptionDebugger for Option<T> {
    fn print_none(&self, msg: &str) {
        match self {
            Some(_) => {}
            None => {
                log::error!("{msg} None");
            }
        }
    }
}
