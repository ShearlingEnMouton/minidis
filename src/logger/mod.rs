use std::collections::HashMap;
use std::sync::LazyLock;

#[derive(Clone, Copy, PartialEq, PartialOrd)]
#[allow(clippy::upper_case_acronyms)]
pub enum LogLevel {
    ERR,
    WARN,
    INFO,
    DEBUG,
}

static LOG_LEVELS: LazyLock<HashMap<&str, LogLevel>> = LazyLock::new(|| {
    HashMap::from([
        ("ERR", LogLevel::ERR),
        ("WARN", LogLevel::WARN),
        ("INFO", LogLevel::INFO),
        ("DEBUG", LogLevel::DEBUG),
    ])
});

static LEVEL: LazyLock<String> = LazyLock::new(|| {
    let level = match std::env::var("LOG_LEVEL") {
        Ok(level) => level.to_uppercase(),
        Err(_) => {
            println!("LOG_LEVEL not set, defaulting to INFO");
            return String::from("INFO");
        }
    };

    if !LOG_LEVELS.contains_key(level.as_str()) {
        println!(
            "Invalid log level: {}. Available log levels are ERR, WARN, INFO, DEBUG. Using default log level INFO",
            level
        );
        return String::from("INFO");
    }

    level
});

pub static CURRENT_LEVEL: LazyLock<LogLevel> = LazyLock::new(|| LOG_LEVELS[LEVEL.as_str()]);

pub fn timestamp() -> String {
    chrono::Local::now()
        .format("%Y/%m/%d %H:%M:%S%.6f")
        .to_string()
}

#[macro_export]
macro_rules! log_err {
    ($($arg:tt)*) => {
        if *$crate::logger::CURRENT_LEVEL >= $crate::logger::LogLevel::ERR {
            println!("{} [ERR] {}", $crate::logger::timestamp(), format!($($arg)*));
        }
    };
}

#[macro_export]
macro_rules! log_warn {
    ($($arg:tt)*) => {
        if *$crate::logger::CURRENT_LEVEL >= $crate::logger::LogLevel::WARN {
            println!("{} [WARN] {}", $crate::logger::timestamp(), format!($($arg)*));
        }
    };
}

#[macro_export]
macro_rules! log_info {
    ($($arg:tt)*) => {
        if *$crate::logger::CURRENT_LEVEL >= $crate::logger::LogLevel::INFO {
            println!("{} [INFO] {}", $crate::logger::timestamp(), format!($($arg)*));
        }
    };
}

#[macro_export]
macro_rules! log_debug {
    ($($arg:tt)*) => {
        if *$crate::logger::CURRENT_LEVEL >= $crate::logger::LogLevel::DEBUG {
            println!("{} [DEBUG] {}", $crate::logger::timestamp(), format!($($arg)*));
        }
    };
}
