use std::fs;
use std::os::unix::fs::MetadataExt;
use std::path::Path;

use anyhow::Result;
use irixmail_core::BootstrapConfig;

const SERVICE_USER: &str = "irixmail";

pub fn ensure_service_ownership(config: &BootstrapConfig, config_file: &Path) -> Result<()> {
    if !running_as_root() {
        return Ok(());
    }
    let Ok(passwd) = fs::read_to_string("/etc/passwd") else {
        return Ok(());
    };
    let Some((uid, gid)) = passwd_ids(&passwd, SERVICE_USER) else {
        return Ok(());
    };
    let targets = [
        config_file.to_path_buf(),
        config.paths.db.clone(),
        config.paths.blobs.clone(),
        config.paths.logs.clone(),
        config.paths.secret_key.clone(),
        crate::setup_cert::certs_dir(config),
    ];
    for path in &targets {
        chown_tree(path, uid, gid)?;
    }
    println!("File ownership set to the {SERVICE_USER} service user.");
    Ok(())
}

pub(crate) fn running_as_root() -> bool {
    fs::metadata("/proc/self")
        .map(|meta| meta.uid() == 0)
        .unwrap_or(false)
}

fn passwd_ids(passwd: &str, user: &str) -> Option<(u32, u32)> {
    passwd.lines().find_map(|line| {
        let mut fields = line.split(':');
        if fields.next()? != user {
            return None;
        }
        let _password = fields.next()?;
        let uid = fields.next()?.parse().ok()?;
        let gid = fields.next()?.parse().ok()?;
        Some((uid, gid))
    })
}

fn chown_tree(path: &Path, uid: u32, gid: u32) -> Result<()> {
    use anyhow::Context;
    if !path.exists() {
        return Ok(());
    }
    std::os::unix::fs::chown(path, Some(uid), Some(gid))
        .with_context(|| format!("changing the owner of {}", path.display()))?;
    if path.is_dir() {
        for entry in fs::read_dir(path).with_context(|| format!("listing {}", path.display()))? {
            chown_tree(&entry?.path(), uid, gid)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const PASSWD: &str = "root:x:0:0:root:/root:/bin/bash\n\
        daemon:x:1:1:daemon:/usr/sbin:/usr/sbin/nologin\n\
        irixmail:x:998:997::/var/lib/irixmail:/usr/sbin/nologin\n";

    #[test]
    fn the_service_user_ids_are_read_from_passwd() {
        assert_eq!(passwd_ids(PASSWD, "irixmail"), Some((998, 997)));
        assert_eq!(passwd_ids(PASSWD, "root"), Some((0, 0)));
    }

    #[test]
    fn a_missing_or_malformed_user_yields_none() {
        assert_eq!(passwd_ids(PASSWD, "postfix"), None);
        assert_eq!(
            passwd_ids("irixmail:x:not-a-uid:997::/:/bin/sh\n", "irixmail"),
            None
        );
        assert_eq!(passwd_ids("", "irixmail"), None);
    }

    #[test]
    fn a_tree_is_chowned_recursively_to_the_current_owner() {
        let root = std::env::temp_dir().join(format!("irixmail-chown-{}", std::process::id()));
        let nested = root.join("inner");
        fs::create_dir_all(&nested).unwrap();
        fs::write(nested.join("file"), b"x").unwrap();
        let meta = fs::metadata(&root).unwrap();
        chown_tree(&root, meta.uid(), meta.gid()).expect("chown to self succeeds");
        chown_tree(&root.join("missing"), meta.uid(), meta.gid()).expect("missing path is a no-op");
        let _ = fs::remove_dir_all(&root);
    }
}
