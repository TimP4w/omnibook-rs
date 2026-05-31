use std::ffi::OsStr;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read};
use std::os::unix::ffi::OsStrExt;
use std::os::unix::io::AsRawFd;
use std::path::{Path, PathBuf};

const REPORT_ID_INTENSITY: u8 = 0x37;
const VENDOR_ID: &str = "06CB";
const PRODUCT_ID: &str = "CFD2";
const HIDIOCSFEATURE_9: u64 = 0xC0094806;

#[derive(Clone)]
pub struct HapticDevice {
    device_path: Option<PathBuf>,
}

impl HapticDevice {
    pub fn new() -> Self {
        let mut dev = Self { device_path: None };
        dev.find_device();
        dev
    }

    pub fn find_device(&mut self) -> bool {
        let mut found = None;

        if let Ok(entries) = fs::read_dir("/dev") {
            for entry in entries.flatten() {
                let path = entry.path();
                if !path
                    .file_name()
                    .map_or(false, |n| n.as_bytes().starts_with(b"hidraw"))
                {
                    continue;
                }

                let hidraw_num = match path.file_name().and_then(OsStr::to_str) {
                    Some(name) => name.trim_start_matches("hidraw"),
                    None => continue,
                };

                let uevent_path = format!("/sys/class/hidraw/hidraw{}/device/uevent", hidraw_num);
                if !Path::new(&uevent_path).is_file() {
                    continue;
                }

                let mut contents = String::new();
                if File::open(&uevent_path)
                    .and_then(|mut f| f.read_to_string(&mut contents))
                    .is_err()
                {
                    continue;
                }

                if contents.contains(VENDOR_ID) && contents.contains(PRODUCT_ID) {
                    found = Some(path);
                    break;
                }
            }
        }

        self.device_path = found;
        self.device_path.is_some()
    }

    pub fn set_intensity(&self, intensity: u8) -> io::Result<bool> {
        let device_path = self
            .device_path
            .as_ref()
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "Device not found"))?;

        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .open(device_path)?;

        let mut buf = [0u8; 9];
        buf[0] = REPORT_ID_INTENSITY;
        buf[1] = intensity;

        unsafe {
            let ret = libc::ioctl(file.as_raw_fd(), HIDIOCSFEATURE_9, buf.as_mut_ptr());
            if ret < 0 {
                return Err(io::Error::last_os_error());
            }
        }

        Ok(true)
    }

    pub fn get_device_path(&self) -> Option<String> {
        return self
            .device_path
            .as_ref()
            .map(|path| path.display().to_string());
    }
}
