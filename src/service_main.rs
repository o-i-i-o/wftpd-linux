#![allow(dead_code)]

use wftpg::AppState;

fn main() {
    println!("WFTPG Service - SFTP/FTP Server");
    println!("Starting servers...");
    
    let state = AppState::new();
    
    if let Err(e) = state.start_all() {
        eprintln!("Failed to start servers: {}", e);
        std::process::exit(1);
    }
    
    println!("Servers started. Press Ctrl+C to stop.");
    
    ctrlc::set_handler(move || {
        println!("\nStopping servers...");
        state.stop_all();
        std::process::exit(0);
    }).expect("Error setting Ctrl-C handler");
    
    std::thread::park();
}
