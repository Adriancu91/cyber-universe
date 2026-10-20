//! A file system that lives in memory (the heap): the root file system, filled from the initrd at boot.

use super::vfs::{FileSystem, FsError, Kind};
use alloc::{collections::BTreeMap, string::String, vec::Vec};

enum Node {
    File(Vec<u8>),
    Dir(BTreeMap<String, Node>),
}

pub struct RamFs {
    root: crate::sched::Mutex<Node>,
}

impl RamFs {
    pub fn new() -> Self {
        Self { root: crate::sched::Mutex::new(Node::Dir(BTreeMap::new())) }
    }
}

fn find<'a>(mut node: &'a Node, path: &[&str]) -> Result<&'a Node, FsError> {
    for c in path {
        match node {
            Node::Dir(children) => node = children.get(*c).ok_or(FsError::NotFound)?,
            Node::File(_) => return Err(FsError::NotADirectory),
        }
    }
    Ok(node)
}

fn find_mut<'a>(mut node: &'a mut Node, path: &[&str]) -> Result<&'a mut Node, FsError> {
    for c in path {
        match node {
            Node::Dir(children) => node = children.get_mut(*c).ok_or(FsError::NotFound)?,
            Node::File(_) => return Err(FsError::NotADirectory),
        }
    }
    Ok(node)
}

/// The directory that holds `path`, and the last component's name.
fn parent<'a, 'p>(root: &'a mut Node, path: &[&'p str]) -> Result<(&'a mut BTreeMap<String, Node>, &'p str), FsError> {
    let (name, dir) = path.split_last().ok_or(FsError::InvalidPath)?;
    match find_mut(root, dir)? {
        Node::Dir(children) => Ok((children, name)),
        Node::File(_) => Err(FsError::NotADirectory),
    }
}

impl FileSystem for RamFs {
    fn kind(&self, path: &[&str]) -> Result<Kind, FsError> {
        Ok(match find(&self.root.lock(), path)? {
            Node::File(_) => Kind::File,
            Node::Dir(_) => Kind::Dir,
        })
    }

    fn size(&self, path: &[&str]) -> Result<u64, FsError> {
        match find(&self.root.lock(), path)? {
            Node::File(d) => Ok(d.len() as u64),
            Node::Dir(c) => Ok(c.len() as u64),
        }
    }

    fn read(&self, path: &[&str], offset: u64, buf: &mut [u8]) -> Result<usize, FsError> {
        match find(&self.root.lock(), path)? {
            Node::File(data) => {
                let start = (offset as usize).min(data.len());
                let n = buf.len().min(data.len() - start);
                buf[..n].copy_from_slice(&data[start..start + n]);
                Ok(n)
            }
            Node::Dir(_) => Err(FsError::IsADirectory),
        }
    }

    fn write(&self, path: &[&str], offset: u64, bytes: &[u8]) -> Result<usize, FsError> {
        match find_mut(&mut self.root.lock(), path)? {
            Node::File(data) => {
                let end = offset as usize + bytes.len();
                if data.len() < end {
                    data.resize(end, 0);
                }
                data[offset as usize..end].copy_from_slice(bytes);
                Ok(bytes.len())
            }
            Node::Dir(_) => Err(FsError::IsADirectory),
        }
    }

    fn create(&self, path: &[&str], kind: Kind) -> Result<(), FsError> {
        let mut root = self.root.lock();
        let (dir, name) = parent(&mut root, path)?;
        if dir.contains_key(name) {
            return Err(FsError::AlreadyExists);
        }
        let node = match kind {
            Kind::File => Node::File(Vec::new()),
            Kind::Dir => Node::Dir(BTreeMap::new()),
        };
        dir.insert(String::from(name), node);
        Ok(())
    }

    fn list(&self, path: &[&str]) -> Result<Vec<(String, Kind)>, FsError> {
        match find(&self.root.lock(), path)? {
            Node::Dir(children) => Ok(children
                .iter()
                .map(|(n, c)| (n.clone(), if matches!(c, Node::Dir(_)) { Kind::Dir } else { Kind::File }))
                .collect()),
            Node::File(_) => Err(FsError::NotADirectory),
        }
    }

    fn remove(&self, path: &[&str]) -> Result<(), FsError> {
        let mut root = self.root.lock();
        let (dir, name) = parent(&mut root, path)?;
        match dir.get(name).ok_or(FsError::NotFound)? {
            Node::Dir(c) if !c.is_empty() => return Err(FsError::NotEmpty),
            _ => {}
        }
        dir.remove(name);
        Ok(())
    }
}
