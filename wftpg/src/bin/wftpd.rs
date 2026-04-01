fn main() {
    if let Err(e) = wftpg::service::run_service() {
        eprintln!("Service error: {}", e);
        std::process::exit(1);
    }
}
