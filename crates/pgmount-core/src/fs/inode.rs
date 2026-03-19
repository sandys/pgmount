use dashmap::DashMap;
use std::sync::atomic::{AtomicU64, Ordering};

/// Identifies a unique virtual node in the filesystem tree.
/// Same identity = same inode within a mount session.
#[derive(Debug, Clone, Hash, PartialEq, Eq)]
pub enum NodeIdentity {
    Root,
    Schema { name: String },
    Table { schema: String, table: String },
    /// Special directories under a table (like .filter, .order, .info, .export, etc.)
    SpecialDir { schema: String, table: String, kind: SpecialDirKind },
    /// A row directory named by its PK value(s)
    Row { schema: String, table: String, pk_display: String },
    /// A column file inside a row directory
    Column { schema: String, table: String, pk_display: String, column: String },
    /// Full-row file in a format (row.json, row.csv, row.yaml)
    RowFile { schema: String, table: String, pk_display: String, format: String },
    /// Filter directory stages: .filter/ -> .filter/<col>/ -> .filter/<col>/<value>/
    FilterDir { schema: String, table: String, stage: FilterStage },
    /// Order directory stages
    OrderDir { schema: String, table: String, stage: OrderStage },
    /// .first/<N>/ or .last/<N>/ directories
    LimitDir { schema: String, table: String, kind: LimitKind, n: u64 },
    /// .by/<column>/<value>/ index lookup
    ByIndexDir { schema: String, table: String, stage: ByIndexStage },
    /// Files inside .info/ directory
    InfoFile { schema: String, table: String, filename: String },
    /// Files inside .export/ directory
    ExportFile { schema: String, table: String, format: String },
    /// .indexes/ directory and children
    IndexDir { schema: String, table: String },
    IndexFile { schema: String, table: String, index_name: String },
    /// .views/ directory and children
    ViewsDir { schema: String },
    View { schema: String, view_name: String },
}

#[derive(Debug, Clone, Hash, PartialEq, Eq)]
pub enum SpecialDirKind {
    Filter,
    Order,
    FirstLast,
    Info,
    Export,
    Import,
    Indexes,
    By,
    Sample,
}

#[derive(Debug, Clone, Hash, PartialEq, Eq)]
pub enum FilterStage {
    Root,                                    // .filter/
    Column { column: String },               // .filter/<col>/
    Value { column: String, value: String },  // .filter/<col>/<value>/
}

#[derive(Debug, Clone, Hash, PartialEq, Eq)]
pub enum OrderStage {
    Root,                                    // .order/
    Column { column: String },               // .order/<col>/
    Direction { column: String, dir: String }, // .order/<col>/asc or desc/
}

#[derive(Debug, Clone, Hash, PartialEq, Eq)]
pub enum LimitKind {
    First,
    Last,
}

#[derive(Debug, Clone, Hash, PartialEq, Eq)]
pub enum ByIndexStage {
    Root,                                     // .by/
    Column { column: String },                // .by/<col>/
    Value { column: String, value: String },   // .by/<col>/<value>/
}

/// Maps NodeIdentity -> inode number and vice versa.
pub struct InodeTable {
    identity_to_ino: DashMap<NodeIdentity, u64>,
    ino_to_identity: DashMap<u64, NodeIdentity>,
    next_ino: AtomicU64,
}

impl Default for InodeTable {
    fn default() -> Self {
        Self::new()
    }
}

impl InodeTable {
    pub fn new() -> Self {
        let table = Self {
            identity_to_ino: DashMap::new(),
            ino_to_identity: DashMap::new(),
            next_ino: AtomicU64::new(2), // 1 is reserved for root
        };
        // Pre-register root
        table.identity_to_ino.insert(NodeIdentity::Root, 1);
        table.ino_to_identity.insert(1, NodeIdentity::Root);
        table
    }

    /// Get or allocate an inode for the given identity.
    pub fn get_or_insert(&self, identity: NodeIdentity) -> u64 {
        if let Some(ino) = self.identity_to_ino.get(&identity) {
            return *ino;
        }
        let ino = self.next_ino.fetch_add(1, Ordering::Relaxed);
        self.identity_to_ino.insert(identity.clone(), ino);
        self.ino_to_identity.insert(ino, identity);
        ino
    }

    /// Look up identity by inode number.
    pub fn get_identity(&self, ino: u64) -> Option<NodeIdentity> {
        self.ino_to_identity.get(&ino).map(|r| r.clone())
    }

    /// Look up inode by identity.
    pub fn get_ino(&self, identity: &NodeIdentity) -> Option<u64> {
        self.identity_to_ino.get(identity).map(|r| *r)
    }
}
