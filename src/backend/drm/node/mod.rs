//! Module for abstractions on drm device nodes

pub(crate) mod constants;

use constants::*;
use libc::dev_t;

use std::{
    fmt::{self, Display, Formatter},
    io,
    os::unix::io::{AsFd, AsRawFd},
    path::{Path, PathBuf},
};

use nix::sys::stat::{fstat, stat, FileStat};
#[cfg(not(target_os = "freebsd"))]
use nix::sys::stat::{major, minor};

// Not currently provided in `libc` or `nix`
// https://github.com/rust-lang/libc/pull/2999
#[cfg(target_os = "freebsd")]
fn major(dev: dev_t) -> u64 {
    ((dev >> 8) & 0xff) as u64
}

#[cfg(target_os = "freebsd")]
fn minor(dev: dev_t) -> u64 {
    (dev & 0xffff00ff) as u64
}

/// A node which refers to a DRM device.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct DrmNode {
    dev: dev_t,
    ty: NodeType,
}

impl DrmNode {
    /// Creates a DRM node from an open drm device.
    ///
    /// This function does not take ownership of the passed in file descriptor.
    pub fn from_file<A: AsFd>(file: A) -> Result<DrmNode, CreateDrmNodeError> {
        let stat = fstat(file.as_fd().as_raw_fd()).map_err(Into::<io::Error>::into)?;
        DrmNode::from_stat(stat)
    }

    /// Creates a DRM node from path.
    pub fn from_path<A: AsRef<Path>>(path: A) -> Result<DrmNode, CreateDrmNodeError> {
        dbg!("from_path", path.as_ref());
        let stat = stat(path.as_ref()).map_err(Into::<io::Error>::into)?;
        DrmNode::from_stat(stat)
    }

    /// Creates a DRM node from a file stat.
    pub fn from_stat(stat: FileStat) -> Result<DrmNode, CreateDrmNodeError> {
        let dev = stat.st_rdev;
        DrmNode::from_dev_id(dev)
    }

    /// Creates a DRM node from a dev_t
    pub fn from_dev_id(dev: dev_t) -> Result<DrmNode, CreateDrmNodeError> {
        let major = major(dev);
        let minor = minor(dev);

        dbg!("from_dev_id", dev, major, minor);

        if !is_device_drm(major, minor) {
            return Err(CreateDrmNodeError::NotDrmNode);
        }

        /*
        The type of the DRM node is determined by the node number ranges.

        0-63 -> Primary
        64-127 -> Control
        128-255 -> Render
        */
        let Some(id) = node_id(major, minor) else {
            return Err(CreateDrmNodeError::NotDrmNode);
        };
        dbg!(id);
        let ty = match id >> 6 {
            0 => NodeType::Primary,
            1 => NodeType::Control,
            2 => NodeType::Render,
            _ => return Err(CreateDrmNodeError::NotDrmNode),
        };

        Ok(DrmNode { dev, ty })
    }

    /// Returns the type of the DRM node.
    pub fn ty(&self) -> NodeType {
        self.ty
    }

    /// Returns the device_id of the underlying DRM node.
    pub fn dev_id(&self) -> dev_t {
        self.dev
    }

    /// Returns the path of the open device if possible.
    pub fn dev_path(&self) -> Option<PathBuf> {
        node_path(self, self.ty).ok()
    }

    /// Returns the path of the specified node type matching the device, if available.
    pub fn dev_path_with_type(&self, ty: NodeType) -> Option<PathBuf> {
        node_path(self, ty).ok()
    }

    /// Returns a new node of the specified node type matching the device, if available.
    pub fn node_with_type(&self, ty: NodeType) -> Option<Result<DrmNode, CreateDrmNodeError>> {
        self.dev_path_with_type(ty).map(DrmNode::from_path)
    }

    /// Returns the major device number of the DRM device.
    pub fn major(&self) -> u64 {
        major(self.dev_id())
    }

    /// Returns the minor device number of the DRM device.
    pub fn minor(&self) -> u64 {
        minor(self.dev_id())
    }

    /// Returns whether the DRM device has render nodes.
    pub fn has_render(&self) -> bool {
        #[cfg(target_os = "linux")]
        {
            node_path(self, NodeType::Render).is_ok()
        }

        // TODO: More robust checks on non-linux.
        #[cfg(target_os = "freebsd")]
        {
            false
        }

        #[cfg(not(any(target_os = "linux", target_os = "freebsd")))]
        {
            false
        }
    }
}

impl Display for DrmNode {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        let major = major(self.dev_id());
        let minor = minor(self.dev_id());
        let id = node_id(major, minor).unwrap(); // XXX
        write!(f, "{}{}", self.ty.minor_name_prefix(), id)
    }
}

/// A type of node
#[derive(Debug, Clone, Copy, Hash, PartialEq, Eq)]
pub enum NodeType {
    /// A primary node may be used to allocate buffers.
    ///
    /// If no other node is present, this may be used to post a buffer to an output with mode-setting.
    Primary,

    /// A control node may be used for mode-setting.
    ///
    /// This is almost never used since no DRM API for control nodes is available yet.
    Control,

    /// A render node may be used by a client to allocate buffers.
    ///
    /// Mode-setting is not possible with a render node.
    Render,
}

impl NodeType {
    /// Returns a string representing the prefix of a minor device's name.
    ///
    /// For example, on Linux with a primary node, the returned string would be `card`.
    pub fn minor_name_prefix(&self) -> &str {
        match self {
            NodeType::Primary => PRIMARY_NAME,
            NodeType::Control => CONTROL_NAME,
            NodeType::Render => RENDER_NAME,
        }
    }
}

impl Display for NodeType {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{}",
            match self {
                NodeType::Primary => "Primary",
                NodeType::Control => "Control",
                NodeType::Render => "Render",
            }
        )
    }
}

/// An error that may occur when creating a DrmNode from a file descriptor.
#[derive(Debug, thiserror::Error)]
pub enum CreateDrmNodeError {
    /// Some underlying IO error occured while trying to create a DRM node.
    #[error("{0}")]
    Io(io::Error),

    /// The provided file descriptor does not refer to a DRM node.
    #[error("the provided file descriptor does not refer to a DRM node.")]
    NotDrmNode,
}

impl From<io::Error> for CreateDrmNodeError {
    fn from(err: io::Error) -> Self {
        CreateDrmNodeError::Io(err)
    }
}

/// Returns if the given device by major:minor pair is a drm device
#[cfg(target_os = "linux")]
pub fn is_device_drm(major: u64, minor: u64) -> bool {
    let path = format!("/sys/dev/char/{}:{}/device/drm", major, minor);
    stat(path.as_str()).is_ok()
}

#[cfg(target_os = "freebsd")]
fn devname(major: u64, minor: u64) -> Option<String> {
    use nix::sys::stat::SFlag;
    use std::os::raw::{c_char, c_int};

    // Matching value of SPECNAMELEN in FreeBSD 13+
    let mut dev_name = vec![0u8; 255];

    let buf: *mut c_char = unsafe {
        libc::devname_r(
            libc::makedev(major as _, minor as _),
            SFlag::S_IFCHR.bits(), // Must be S_IFCHR or S_IFBLK
            dev_name.as_mut_ptr() as *mut c_char,
            dev_name.len() as c_int,
        )
    };

    // Buffer was too small (weird issue with the size of buffer) or the device could not be named.
    if buf.is_null() {
        return None;
    }

    // SAFETY: The buffer written to by devname_r is guaranteed to be NUL terminated.
    unsafe { dev_name.set_len(libc::strlen(buf)) };

    Some(String::from_utf8(dev_name).expect("Returned device name is not valid utf8"))
}

/// Returns if the given device by major:minor pair is a drm device
#[cfg(target_os = "freebsd")]
pub fn is_device_drm(major: u64, minor: u64) -> bool {
    devname(major, minor).map_or(false, |dev_name| {
        dev_name.starts_with("drm/")
            || dev_name.starts_with("dri/card")
            || dev_name.starts_with("dri/control")
            || dev_name.starts_with("dri/renderD")
    })
}

/// Returns if the given device by major:minor pair is a drm device
#[cfg(not(any(target_os = "linux", target_os = "freebsd")))]
pub fn is_device_drm(major: u64, _minor: u64) -> bool {
    major == DRM_MAJOR
}

/// Returns the path of a specific type of node from the same DRM device as another path of the same node.
#[cfg(any(target_os = "linux", target_os = "freebsd"))]
pub fn path_to_type<P: AsRef<Path>>(path: P, ty: NodeType) -> io::Result<PathBuf> {
    let stat = stat(path.as_ref()).map_err(Into::<io::Error>::into)?;
    let dev = stat.st_rdev;
    let major = major(dev);
    let minor = minor(dev);

    dev_path(major, minor, ty)
}

/// Returns the path of a specific type of node from the same DRM device as an existing DrmNode.
#[cfg(any(target_os = "linux", target_os = "freebsd"))]
pub fn node_path(node: &DrmNode, ty: NodeType) -> io::Result<PathBuf> {
    let major = node.major();
    let minor = node.minor();

    dev_path(major, minor, ty)
}

/// Returns the path of a specific type of node from the DRM device described by major and minor device numbers.
#[cfg(target_os = "linux")]
pub fn dev_path(major: u64, minor: u64, ty: NodeType) -> io::Result<PathBuf> {
    use std::fs;
    use std::io::ErrorKind;

    if !is_device_drm(major, minor) {
        return Err(io::Error::new(
            ErrorKind::NotFound,
            format!("{}:{} is no DRM device", major, minor),
        ));
    }

    let read = fs::read_dir(format!("/sys/dev/char/{}:{}/device/drm", major, minor))?;

    for entry in read.flatten() {
        let name = entry.file_name();
        let name = name.to_string_lossy();

        // Only 1 primary, control and render node may exist simultaneously, so the
        // first occurrence is good enough.
        if name.starts_with(ty.minor_name_prefix()) {
            let path = [r"/", "dev", "dri", &name].iter().collect::<PathBuf>();
            if path.exists() {
                return Ok(path);
            }
        }
    }

    Err(io::Error::new(
        ErrorKind::NotFound,
        format!(
            "Could not find node of type {} from DRM device {}:{}",
            ty, major, minor
        ),
    ))
}

/// Returns the path of a specific type of node from the DRM device described by major and minor device numbers.
#[cfg(target_os = "freebsd")]
fn dev_path(major: u64, minor: u64, ty: NodeType) -> io::Result<PathBuf> {
    // Based on libdrm `drmGetMinorNameForFD`. Should be updated if the code
    // there is replaced with anything more sensible...

    use std::io::ErrorKind;

    if !is_device_drm(major, minor) {
        return Err(io::Error::new(
            ErrorKind::NotFound,
            format!("{}:{} is no DRM device", major, minor),
        ));
    }

    if let Some(old_id) = node_id(major, minor) {
        let old_ty = match old_id >> 6 {
            0 => NodeType::Primary,
            1 => NodeType::Control,
            2 => NodeType::Render,
            _ => {
                return Err(io::Error::new(
                    ErrorKind::NotFound,
                    format!("{}:{} is no DRM device", major, minor),
                ));
            }
        };
        let id = old_id - get_minor_base(old_ty) + get_minor_base(ty);
        dbg!(old_id, get_minor_base(old_ty), get_minor_base(ty), id);
        let path = PathBuf::from(format!("/dev/dri/{}{}", ty.minor_name_prefix(), id));
        if path.exists() {
            return Ok(path);
        }
    }

    Err(io::Error::new(
        ErrorKind::NotFound,
        format!(
            "Could not find node of type {} from DRM device {}:{}",
            ty, major, minor
        ),
    ))
}

#[cfg(target_os = "linux")]
fn node_id(_major: u64, minor: u64) -> Option<u32> {
    Some(minor)
}

#[cfg(target_os = "freebsd")]
fn node_id(major: u64, minor: u64) -> Option<u32> {
    let dev_name = devname(major, minor)?;
    let suffix = dev_name.trim_start_matches(|c: char| !c.is_numeric());
    suffix.parse::<u32>().ok()
}

#[cfg(target_os = "freebsd")]
fn get_minor_base(type_: NodeType) -> u32 {
    match type_ {
        NodeType::Primary => 0,
        NodeType::Control => 64,
        NodeType::Render => 128,
    }
}
