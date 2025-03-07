use std::{
    collections::HashMap,
    ffi::{OsStr, OsString},
    io::{BufRead, BufReader},
    os::unix::ffi::OsStringExt,
    path::{Path, PathBuf},
};

pub struct Enumerator {
    subsystem: Option<String>,
}

const UDEV_ROOT: &str = "/sys";
const DEV_ROOT: &str = "/dev";

impl Enumerator {
    pub fn new() -> Enumerator {
        Enumerator { subsystem: None }
    }

    pub fn match_subsystem(&mut self, subsystem: &str) {
        self.subsystem = Some(subsystem.to_owned());
    }

    pub fn scan_devices(&self) -> std::io::Result<Vec<Device>> {
        let mut devices = vec![];
        let mut device_path = PathBuf::from(UDEV_ROOT);
        device_path.push("class");
        device_path.push(self.subsystem.as_ref().unwrap());
        for dir_entry in device_path.read_dir()? {
            let dir_entry = dir_entry?;

            if let Some(device) = Device::from_syspath(&dir_entry.path()) {
                devices.push(device);
            }
        }
        Ok(devices)
    }
}

#[derive(Debug)]
#[allow(dead_code)]
pub struct Device {
    system_path: PathBuf,
    subsystem: Option<String>,
    driver: Option<String>,
    devnode: Option<PathBuf>,
    attributes: HashMap<String, OsString>,
    properties: HashMap<String, OsString>,
    parent: Option<Box<Device>>,
}

impl Device {
    pub fn from_syspath(path: &Path) -> Option<Device> {
        let path = path.canonicalize().ok()?;
        let mut parent = None;

        // Keep looking for a parent object.
        let mut parent_path = path.parent();
        loop {
            let Some(path) = parent_path else {
                break;
            };
            let uevent = path.join("uevent");
            if uevent.exists() && uevent.is_file() {
                parent = Self::from_syspath(path).map(|parent| Box::new(parent));
                break;
            }
            parent_path = path.parent();
        }

        let uevent = path.join("uevent");
        if !uevent.exists() || !uevent.is_file() {
            return None;
        }

        let mut properties = HashMap::new();
        let Ok(uevent) = std::fs::File::open(uevent) else {
            return None;
        };
        for line in BufReader::new(uevent).lines() {
            let Ok(line) = line else {
                break;
            };
            let Some((key, value)) = line.split_once('=') else {
                continue;
            };
            properties.insert(
                key.to_owned(),
                OsString::from_vec(value.as_bytes().to_vec()),
            );
        }

        let subsystem = path.join("subsystem").canonicalize().ok().and_then(|dir| {
            dir.file_name()
                .and_then(|v| v.to_str())
                .map(|name| name.to_string())
        });
        let driver = path.join("driver").canonicalize().ok().and_then(|dir| {
            dir.file_name()
                .and_then(|v| v.to_str())
                .map(|name| name.to_string())
        });

        let mut attrs = HashMap::new();
        Self::update_attrs(&path, &path, &mut attrs);

        // TODO: What if /dev isn't where things are mounted?
        let mut devnode = None;
        if let Some(devname) = properties.get("DEVNAME") {
            let mut tmp_dev = Path::new(DEV_ROOT).to_path_buf();
            tmp_dev.push(devname);
            devnode = Some(tmp_dev)
        }

        Some(Device {
            system_path: path.to_owned(),
            subsystem,
            driver,
            attributes: attrs,
            parent,
            properties,
            devnode,
        })
    }


    fn update_attrs(path: &Path, base: &Path, attrs: &mut HashMap<String, OsString>) {
        let Ok(read_dir) = path.read_dir() else {
            return;
        };
        for entry in read_dir {
            let Ok(entry) = entry else {
                continue;
            };
            let entry_path = entry.path();
            if entry_path.is_symlink() {
                continue;
            }
            let entry_name = entry_path.file_name().and_then(|v| v.to_str());
            if entry_name == Some("uevent") || entry_name == Some("dev") {
                continue;
            }

            if entry_path.is_dir() {
                let mut sub_dir = entry_path.clone();
                sub_dir.push("uevent");
                if !sub_dir.exists() {
                    Self::update_attrs(&entry_path, base, attrs);
                    continue;
                }
            }

            if entry_path.is_file() {
                let Some(name) = entry_path
                    .strip_prefix(base)
                    .ok()
                    .map(|name| name.as_os_str())
                    .map(|name| name.to_str())
                    .flatten()
                    .map(|v| v.to_owned())
                else {
                    continue;
                };
                let Ok(value) = std::fs::read(entry_path) else {
                    continue;
                };
                attrs.insert(name, OsString::from_vec(value));
                continue;
            }
        }
    }

    pub fn parent_with_subsystem(&self, parent_subsystem: &str) -> Option<&Device> {
        let mut parent = &self.parent;
        loop {
            let Some(unwrapped_parent) = parent else {
                return None;
            };

            if unwrapped_parent.subsystem.as_deref() == Some(parent_subsystem) {
                return Some(&unwrapped_parent);
            }

            parent = &unwrapped_parent.parent;
        }
    }

    pub fn parent_with_subsystem_devtype(
        &self,
        parent_subsystem: &str,
        devtype: &str,
    ) -> Option<&Device> {
        let mut parent = &self.parent;
        loop {
            let Some(unwrapped_parent) = parent else {
                return None;
            };

            if unwrapped_parent.subsystem.as_deref() == Some(parent_subsystem)
                && unwrapped_parent
                    .attribute_value("DEVTYPE")
                    .map(|x| x.to_str())
                    .flatten()
                    == Some(devtype)
            {
                return Some(&unwrapped_parent);
            }

            parent = &unwrapped_parent.parent;
        }
    }

    pub fn property_value(&self, key: &str) -> Option<&OsStr> {
        self.properties.get(key).map(|x| x.as_os_str())
    }

    pub fn attribute_value(&self, key: &str) -> Option<&OsStr> {
        self.attributes.get(key).map(|x| x.as_os_str())
    }

    pub fn devnode(&self) -> Option<&Path> {
        self.devnode.as_deref()
    }

    pub fn syspath(&self) -> &Path {
        self.system_path.as_path()
    }
}
