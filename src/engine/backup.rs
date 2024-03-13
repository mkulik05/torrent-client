use crate::engine::logger::{log, LogLevel};
use crate::gui::TorrentBackupInfo;
use dirs::data_local_dir;
use std::fs::File;
use std::io::{Read, Seek, Write};
use std::path::PathBuf;

const BACKUP_NAME: &str = "backup.bin";

fn get_conf_path() -> anyhow::Result<PathBuf> {
    let config_dir = data_local_dir();
    if let Some(dir) = config_dir {
        return Ok(dir.to_owned().join(BACKUP_NAME));
    }
    log!(LogLevel::Error, "Could not get config folder path");
    anyhow::bail!("Failed to get config folder path");
}

pub fn load_config() -> anyhow::Result<Vec<TorrentBackupInfo>> {
    let path = get_conf_path()?;
    let mut file = File::open(path)?;
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes)?;
    Ok(bincode::deserialize(&bytes)?)
}

pub fn backup_torrent(data: TorrentBackupInfo) -> anyhow::Result<()> {
    let path = get_conf_path()?;
    let mut file = File::options()
        .create(true)
        .write(true)
        .read(true)
        .open(path)?;
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes)?;

    let mut backups: Vec<TorrentBackupInfo> = bincode::deserialize(&bytes).unwrap_or_else(|_| {
        log!(
            LogLevel::Error,
            "Backup file is damaged, it will be recreated"
        );
        Vec::new()
    });

    let mut replace_i = None;

    for (i, backup) in backups.iter().enumerate() {
        if backup.torrent.info_hash == data.torrent.info_hash {
            replace_i = Some(i);
            break;
        }
    }

    if let Some(i) = replace_i {
        backups[i] = data;
    } else {
        backups.push(data);
    }

    let bytes = bincode::serialize(&backups)?;

    file.seek(std::io::SeekFrom::Start(0))?;
    file.write_all(&bytes)?;
    file.set_len(bytes.len() as u64)?;

    Ok(())
}

pub fn load_backup(info_hash: &Vec<u8>) -> anyhow::Result<TorrentBackupInfo> {
    let path = get_conf_path()?;
    let mut file = File::open(path)?;
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes)?;

    let backups: Vec<TorrentBackupInfo> = bincode::deserialize(&bytes)?;

    for backup in backups {
        if backup.torrent.info_hash == *info_hash {
            return Ok(backup);
        }
    }

    anyhow::bail!("Torrent not found in file");
}

pub fn remove_torrent(info_hash: &Vec<u8>) -> anyhow::Result<()> {
    let path = get_conf_path()?;
    let mut file = File::options()
        .create(true)
        .write(true)
        .read(true)
        .open(path)?;
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes)?;

    let mut backups: Vec<TorrentBackupInfo> = bincode::deserialize(&bytes).unwrap_or_else(|_| {
        log!(
            LogLevel::Error,
            "Backup file is damaged, it will be recreated"
        );
        Vec::new()
    });

    let mut delete_i = None;

    for (i, backup) in backups.iter().enumerate() {
        if backup.torrent.info_hash == *info_hash {
            delete_i = Some(i);
            break;
        }
    }

    if let Some(i) = delete_i {
        backups.remove(i);
        let bytes = bincode::serialize(&backups)?;

        file.seek(std::io::SeekFrom::Start(0))?;
        file.write_all(&bytes)?;
        file.set_len(bytes.len() as u64)?;
    }

    Ok(())
}
