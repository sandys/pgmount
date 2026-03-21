use fuser::{FileAttr, INodeNo};
use std::time::{Duration, SystemTime};

pub const TTL: Duration = Duration::from_secs(1);
pub const BLOCK_SIZE: u32 = 512;

/// Create a FileAttr for a directory.
pub fn dir_attr(ino: u64) -> FileAttr {
    FileAttr {
        ino: INodeNo(ino),
        size: 0,
        blocks: 0,
        atime: SystemTime::now(),
        mtime: SystemTime::now(),
        ctime: SystemTime::now(),
        crtime: SystemTime::now(),
        kind: fuser::FileType::Directory,
        perm: 0o755,
        nlink: 2,
        uid: unsafe { libc::getuid() },
        gid: unsafe { libc::getgid() },
        rdev: 0,
        blksize: BLOCK_SIZE,
        flags: 0,
    }
}

/// Create a FileAttr for a regular file with given size.
pub fn file_attr(ino: u64, size: u64) -> FileAttr {
    FileAttr {
        ino: INodeNo(ino),
        size,
        blocks: size.div_ceil(BLOCK_SIZE as u64),
        atime: SystemTime::now(),
        mtime: SystemTime::now(),
        ctime: SystemTime::now(),
        crtime: SystemTime::now(),
        kind: fuser::FileType::RegularFile,
        perm: 0o444,
        nlink: 1,
        uid: unsafe { libc::getuid() },
        gid: unsafe { libc::getgid() },
        rdev: 0,
        blksize: BLOCK_SIZE,
        flags: 0,
    }
}

/// Create a FileAttr for a writable file.
pub fn writable_file_attr(ino: u64, size: u64) -> FileAttr {
    let mut attr = file_attr(ino, size);
    attr.perm = 0o644;
    attr
}
