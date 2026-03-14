fn main() {
    if let Err(e) = wftpg::service::service_main::run_service() {
        eprintln!("Service error: {}", e);
        std::process::exit(1);
    }
}
