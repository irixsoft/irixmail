use std::net::IpAddr;

use irixmail_directory::Directory;

pub fn is_blocked(directory: &Directory, ip: IpAddr) -> bool {
    directory.ip_rules().blocks(ip)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::sync::Arc;

    use irixmail_core::IdGenerator;
    use irixmail_directory::IpAction;
    use irixmail_store::{RocksdbStore, Store};

    fn directory() -> Directory {
        static COUNTER: AtomicU32 = AtomicU32::new(0);
        let unique = COUNTER.fetch_add(1, Ordering::Relaxed);
        let dir =
            std::env::temp_dir().join(format!("irixmail-ip-guard-{}-{unique}", std::process::id()));
        let store: Arc<dyn Store> = Arc::new(RocksdbStore::open(dir).unwrap());
        Directory::new(store, Arc::new(IdGenerator::new(0)), None)
    }

    #[test]
    fn a_blocked_ip_is_refused_and_an_allowed_override_is_not() {
        let directory = directory();
        directory
            .ip_rules()
            .create("10.0.0.0/8", IpAction::Block)
            .unwrap();
        directory
            .ip_rules()
            .create("10.1.2.3", IpAction::Allow)
            .unwrap();

        assert!(is_blocked(&directory, "10.9.9.9".parse().unwrap()));
        assert!(!is_blocked(&directory, "10.1.2.3".parse().unwrap()));
        assert!(!is_blocked(&directory, "192.0.2.1".parse().unwrap()));
    }

    #[test]
    fn an_ipv4_mapped_ipv6_peer_cannot_bypass_a_v4_block() {
        let directory = directory();
        directory
            .ip_rules()
            .create("10.0.0.0/8", IpAction::Block)
            .unwrap();

        assert!(is_blocked(&directory, "::ffff:10.9.9.9".parse().unwrap()));
        assert!(!is_blocked(&directory, "::ffff:192.0.2.1".parse().unwrap()));
    }
}
