//! The learned dictionary of derived vocabulary words.
//!
//! A [`Vocabulary`] maps derived symbol names (`d001`, `d002`, ...) to a
//! definition: a sequence of capabilities and/or other derived symbols. It
//! supports recursive expansion down to capabilities or down to root O/D/C
//! tokens (both cycle-detected), an audit that every derived word reduces to
//! nothing but roots, and a greedy longest-match compressor that rewrites a
//! raw capability sequence using the vocabulary's derived words.

use std::collections::{BTreeMap, HashSet};

use crate::capability::{is_known_capability, is_root, root_expansion};

pub type Seq = Vec<String>;

#[derive(Debug)]
pub enum VocabError {
    Cycle(String),
    Unknown(String),
}

impl std::fmt::Display for VocabError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            VocabError::Cycle(sym) => write!(f, "cycle detected while expanding `{sym}`"),
            VocabError::Unknown(sym) => write!(f, "unknown symbol `{sym}`"),
        }
    }
}

impl std::error::Error for VocabError {}

#[derive(Clone, Debug, Default)]
pub struct Vocabulary {
    pub derived: BTreeMap<String, Seq>,
    pub counter: usize,
}

impl Vocabulary {
    pub fn new() -> Self {
        Self::default()
    }

    fn expand_symbol_inner(&self, sym: &str, stack: &mut HashSet<String>) -> Result<Seq, VocabError> {
        if is_known_capability(sym) {
            return Ok(vec![sym.to_string()]);
        }
        if !stack.insert(sym.to_string()) {
            return Err(VocabError::Cycle(sym.to_string()));
        }
        let def = self.derived.get(sym).ok_or_else(|| VocabError::Unknown(sym.to_string()))?;
        let mut out = Vec::new();
        for part in def {
            out.extend(self.expand_symbol_inner(part, stack)?);
        }
        stack.remove(sym);
        Ok(out)
    }

    /// Expand a symbol (capability or derived word) down to a flat sequence
    /// of capabilities.
    pub fn expand_symbol(&self, sym: &str) -> Result<Seq, VocabError> {
        self.expand_symbol_inner(sym, &mut HashSet::new())
    }

    /// Expand a symbol all the way down to root O/D/C tokens.
    pub fn expand_root(&self, sym: &str) -> Result<Seq, VocabError> {
        let mut out = Vec::new();
        for cap in self.expand_symbol(sym)? {
            out.extend(root_expansion(&cap).iter().map(|r| (*r).to_string()));
        }
        Ok(out)
    }

    fn depth_inner(&self, sym: &str, stack: &mut HashSet<String>) -> Result<usize, VocabError> {
        if is_known_capability(sym) {
            return Ok(0);
        }
        if !stack.insert(sym.to_string()) {
            return Err(VocabError::Cycle(sym.to_string()));
        }
        let def = self.derived.get(sym).ok_or_else(|| VocabError::Unknown(sym.to_string()))?;
        let mut max_child = 0usize;
        for part in def {
            max_child = max_child.max(self.depth_inner(part, stack)?);
        }
        stack.remove(sym);
        Ok(1 + max_child)
    }

    pub fn depth(&self, sym: &str) -> Result<usize, VocabError> {
        self.depth_inner(sym, &mut HashSet::new())
    }

    /// Every derived word must recursively reduce to nothing but root O/D/C
    /// tokens, with no cycles and no unknown leaves.
    pub fn audit(&self) -> bool {
        self.derived.keys().all(|name| {
            self.expand_root(name)
                .map(|roots| !roots.is_empty() && roots.iter().all(|r| is_root(r)))
                .unwrap_or(false)
        })
    }

    /// All derived words with their capability-level expansions, sorted by
    /// expansion length (longest first) so the compressor prefers the most
    /// specific match.
    pub fn expansions(&self) -> Vec<(String, Seq)> {
        let mut out: Vec<(String, Seq)> = self
            .derived
            .keys()
            .filter_map(|name| self.expand_symbol(name).ok().map(|s| (name.clone(), s)))
            .collect();
        out.sort_by(|a, b| b.1.len().cmp(&a.1.len()).then_with(|| a.0.cmp(&b.0)));
        out
    }

    /// Greedy longest-match compression of `seq`, using both the
    /// vocabulary's own derived words and any caller-supplied `extra`
    /// candidate definitions (used by the planner to score growth
    /// candidates before committing them).
    pub fn compress_with(&self, seq: &[String], mut extra: Vec<(String, Seq)>) -> Seq {
        extra.extend(self.expansions());
        extra.sort_by(|a, b| b.1.len().cmp(&a.1.len()).then_with(|| a.0.cmp(&b.0)));
        let mut out = Vec::new();
        let mut i = 0usize;
        while i < seq.len() {
            let hit = extra
                .iter()
                .find(|(_, ex)| ex.len() >= 2 && i + ex.len() <= seq.len() && seq[i..i + ex.len()] == ex[..]);
            match hit {
                Some((name, ex)) => {
                    out.push(name.clone());
                    i += ex.len();
                }
                None => {
                    out.push(seq[i].clone());
                    i += 1;
                }
            }
        }
        out
    }

    pub fn compress(&self, seq: &[String]) -> Seq {
        self.compress_with(seq, Vec::new())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(xs: &[&str]) -> Seq {
        xs.iter().map(|x| x.to_string()).collect()
    }

    #[test]
    fn derived_word_expands_and_audits() {
        let mut v = Vocabulary::new();
        v.derived.insert("d001".into(), s(&["transform.summarize", "fs.write"]));
        v.derived.insert("d002".into(), s(&["fs.read", "d001"]));
        assert!(v.audit());
        assert_eq!(v.expand_symbol("d002").unwrap(), s(&["fs.read", "transform.summarize", "fs.write"]));
        assert_eq!(v.expand_root("d002").unwrap(), s(&["OBSERVE", "DERIVE", "DERIVE", "COMMIT"]));
        assert_eq!(v.depth("d002").unwrap(), 2);
    }

    #[test]
    fn cycles_fail_audit_not_panic() {
        let mut v = Vocabulary::new();
        v.derived.insert("a".into(), s(&["b"]));
        v.derived.insert("b".into(), s(&["a"]));
        assert!(!v.audit());
        assert!(v.expand_symbol("a").is_err());
    }

    #[test]
    fn unknown_leaf_fails_audit() {
        let mut v = Vocabulary::new();
        v.derived.insert("a".into(), s(&["nonexistent.capability"]));
        assert!(!v.audit());
    }

    #[test]
    fn compress_prefers_longest_match() {
        let mut v = Vocabulary::new();
        v.derived.insert("d001".into(), s(&["fs.read", "transform.summarize"]));
        v.derived.insert("d002".into(), s(&["fs.read", "transform.summarize", "fs.write"]));
        let compressed = v.compress(&s(&["fs.read", "transform.summarize", "fs.write", "notify.send"]));
        assert_eq!(compressed, s(&["d002", "notify.send"]));
    }
}
