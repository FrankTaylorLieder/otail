use std::backtrace::Backtrace;
use std::fs::{File, OpenOptions};
use std::io::Write;
use std::panic;
use std::sync::Mutex;
use std::thread;
use std::time::SystemTime;

// Global static file handle wrapped in a Mutex for thread safety
lazy_static::lazy_static! {
    static ref LOG_FILE: Mutex<File> = Mutex::new(
        OpenOptions::new()
            .create(true)
            .append(true)
            .open("otail-panic.log")
            .expect("Failed to open panic log file")
    );
}

/// Initialize the panic handler
pub fn init_panic_handler() {
    panic::set_hook(Box::new(|panic_info| {
        // Get current time
        let timestamp = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();

        // Capture thread information
        let thread_name = thread::current().name().unwrap_or("<unnamed>").to_string();

        // Get panic location if available
        let location = panic_info
            .location()
            .map(|loc| format!("{}:{}", loc.file(), loc.line()))
            .unwrap_or_else(|| "Unknown location".to_string());

        // Get panic message
        let message = panic_info
            .payload()
            .downcast_ref::<&str>()
            .map(|s| s.to_string())
            .or_else(|| {
                panic_info
                    .payload()
                    .downcast_ref::<String>()
                    .map(|s| s.to_string())
            })
            .unwrap_or_else(|| "Unknown panic message".to_string());

        // Capture backtrace
        let backtrace = Backtrace::capture();

        // Format the panic message
        let panic_message = format!(
            "\n[PANIC] Timestamp: {}\nThread: {}\nLocation: {}\nMessage: {}\nBacktrace:\n{:#?}\n\n",
            timestamp, thread_name, location, message, backtrace
        );

        // Write to log file
        if let Ok(mut file) = LOG_FILE.lock() {
            let _ = file.write_all(panic_message.as_bytes());
            let _ = file.flush();
        }

        // Optionally print to stderr as well
        eprintln!("{}", panic_message);
    }));
}
