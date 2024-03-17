use crate::engine::logger::{log, LogLevel};
use crate::gui::TorrentBackupInfo;
use dirs::data_local_dir;
use std::fs::File;
use std::io::{Read, Seek, Write};
use std::path::PathBuf;
use once_cell::sync::OnceCell;
use tokio::sync::Mutex;
use std::sync::Arc;

static BACKUP: OnceCell<Backup> = OnceCell::new();
const BACKUP_NAME: &str = "backup.bin";

#[derive(Debug)]
pub struct Backup {
    sync: Arc<Mutex<()>>
}

fn get_conf_path() -> anyhow::Result<PathBuf> {
    let config_dir = data_local_dir();
    if let Some(dir) = config_dir {
        return Ok(dir.to_owned().join(BACKUP_NAME));
    }
    log!(LogLevel::Error, "Could not get config folder path");
    anyhow::bail!("Failed to get config folder path");
}

impl Backup {

    pub fn global() -> &'static Backup {
        BACKUP.get().expect("logger is not initialized")
    }

    pub fn init() -> Result<(), anyhow::Error> {
        BACKUP
            .set(Backup { sync: Arc::new(Mutex::new(())) })
            .expect("Failed to initialise logger");

        Ok(())
    }

    pub async fn load_config(&self) -> anyhow::Result<Vec<TorrentBackupInfo>> {
        let _lock = self.sync.lock().await;
        let path = get_conf_path()?;
        let mut file = File::open(path)?;
        let mut bytes = Vec::new();
        file.read_to_end(&mut bytes)?;
        Ok(bincode::deserialize(&bytes)?)
    }
    
    pub async fn backup_torrent(&self, data: TorrentBackupInfo) -> anyhow::Result<()> {
        let _lock = self.sync.lock().await;
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
    
    pub async fn load_backup(&self, info_hash: &Vec<u8>) -> anyhow::Result<TorrentBackupInfo> {
        let _lock = self.sync.lock().await;
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
    
    pub async fn remove_torrent(&self, info_hash: &Vec<u8>) -> anyhow::Result<()> {
        let _lock = self.sync.lock().await;
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
}
