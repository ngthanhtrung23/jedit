use std::{fmt::Display, io::Read, ops::Deref};

use indexmap::IndexMap;
use rayon::iter::{IntoParallelIterator, IntoParallelRefIterator, ParallelIterator};
use serde::Serialize;

use super::INDENT;
use crate::error::{DeserializationError, DumpError, IndexingError, LoadError, MutationError};

struct Selector<'a, T> {
    keys: &'a [T],
    next_key_pos: usize,
}

impl<'a, T: Deref<Target = str>> Selector<'a, T> {
    fn new(keys: &'a [T]) -> Self {
        Self {
            keys,
            next_key_pos: 0,
        }
    }

    fn next(&mut self) -> Option<&str> {
        let res = self.keys.get(self.next_key_pos);
        self.next_key_pos = (self.next_key_pos + 1).min(self.keys.len());
        res.map(Deref::deref)
    }
}

#[derive(Debug, Clone, Copy)]
#[cfg_attr(test, derive(PartialEq))]
pub struct NodeMeta {
    pub n_lines: usize,
    pub n_bytes: usize,
    pub kind: NodeKind,
}

#[derive(Debug, Clone, Copy)]
#[cfg_attr(test, derive(PartialEq))]
pub enum NodeKind {
    Terminal,
    Object,
    Array,
}

impl NodeMeta {
    pub fn null() -> Self {
        NodeMeta {
            n_lines: 1,
            n_bytes: 4,
            kind: NodeKind::Terminal,
        }
    }
}

#[derive(Debug)]
#[cfg_attr(test, derive(PartialEq))]
pub struct Index {
    pub meta: NodeMeta,
    pub kind: IndexKind,
}

#[derive(Debug)]
#[cfg_attr(test, derive(PartialEq))]
pub enum IndexKind {
    Terminal,
    Object(Vec<String>),
    Array(usize),
}

#[derive(Debug)]
#[cfg_attr(test, derive(PartialEq, Clone))]
pub struct Node {
    n_lines: usize,
    n_bytes: usize,
    data: Kind,
}

#[derive(Debug, Clone, Copy)]
#[cfg_attr(test, derive(PartialEq))]
enum Number {
    Int(i64),
    Float(f64),
}

impl Display for Number {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Number::Int(value) => write!(f, "{value}"),
            Number::Float(value) => write!(f, "{value}"),
        }
    }
}

#[derive(Debug)]
#[cfg_attr(test, derive(PartialEq, Clone))]
enum Kind {
    Null,
    Bool(bool),
    Number(Number),
    String(String),
    Array(Vec<Node>),
    Object(IndexMap<String, Node>),
}

impl Kind {
    fn node_kind(&self) -> NodeKind {
        match self {
            Self::Null | Self::Bool(_) | Self::Number(_) | Self::String(_) => NodeKind::Terminal,
            Self::Array(_) => NodeKind::Array,
            Self::Object(_) => NodeKind::Object,
        }
    }
}

#[derive(Debug)]
pub enum AddNodeKey {
    Array,
    Object(String),
}

#[derive(Debug)]
pub enum NodeMutation<'a> {
    Replace(Node),
    Delete(&'a str),
    Append {
        after: &'a str,
        key: AddNodeKey,
        node: Node,
    },
    Rename {
        before: &'a str,
        after: String,
    },
}

/// Strips JSON-insignificant whitespace (spaces, tabs, CRs, LFs) outside of string
/// literals. `sonic_rs` refuses to parse input over 4 GB (see `sonic_rs::Value`'s 32-bit
/// tape representation), so pre-stripping padding whitespace lets us load pretty-printed
/// files that are over that limit only because of indentation/newlines.
fn strip_insignificant_whitespace(mut reader: impl Read) -> Result<Vec<u8>, std::io::Error> {
    let mut output = Vec::new();
    let mut in_string = false;
    let mut escaped = false;
    let mut buf = [0u8; 64 * 1024];

    loop {
        let n = reader.read(&mut buf)?;
        if n == 0 {
            break;
        }

        for &b in &buf[..n] {
            if in_string {
                output.push(b);
                if escaped {
                    escaped = false;
                } else if b == b'\\' {
                    escaped = true;
                } else if b == b'"' {
                    in_string = false;
                }
            } else {
                match b {
                    b' ' | b'\t' | b'\n' | b'\r' => {}
                    b'"' => {
                        in_string = true;
                        output.push(b);
                    }
                    _ => output.push(b),
                }
            }
        }
    }

    Ok(output)
}

impl Node {
    pub fn load(reader: impl std::io::Read) -> Result<Self, LoadError> {
        let stripped = strip_insignificant_whitespace(reader)?;
        let value: serde_json::Value = sonic_rs::from_reader(&stripped[..])?;
        Self::from_serde_json(value).map_err(Into::into)
    }

    pub fn to_string_pretty(&self) -> Result<String, DumpError> {
        sonic_rs::to_string_pretty(self).map_err(Into::into)
    }

    pub fn subtree<T: Deref<Target = str>>(&self, selector: &[T]) -> Result<&Node, IndexingError> {
        self.subtree_inner(Selector::new(selector))
    }

    pub fn metas<T: Deref<Target = str>>(
        &self,
        selector: &[T],
    ) -> Result<Vec<NodeMeta>, IndexingError> {
        let mut metas = Vec::new();
        self.metas_inner(Selector::new(selector), &mut metas)?;
        Ok(metas)
    }

    pub fn replace<T: Deref<Target = str>>(
        &mut self,
        selector: &[T],
        node: Node,
    ) -> Result<Node, MutationError> {
        self.mutate(Selector::new(selector), NodeMutation::Replace(node))
            .map(|res| res.expect("replace mutation should return the old node"))
    }

    pub fn delete<T: Deref<Target = str>>(
        &mut self,
        selector: &[T],
    ) -> Result<Node, MutationError> {
        let len = selector.len();
        if len == 0 {
            return Err(IndexingError::NotIndexable.into());
        }

        self.mutate(
            Selector::new(&selector[..len - 1]),
            NodeMutation::Delete(selector[len - 1].deref()),
        )
        .map(|res| res.expect("delete mutation should return the old node"))
    }

    pub fn append_after<T: Deref<Target = str>>(
        &mut self,
        selector: &[T],
        key: AddNodeKey,
        node: Node,
    ) -> Result<(), MutationError> {
        let len = selector.len();
        if len == 0 {
            return Err(IndexingError::NotIndexable.into());
        }

        self.mutate(
            Selector::new(&selector[..len - 1]),
            NodeMutation::Append {
                after: selector[len - 1].deref(),
                key,
                node,
            },
        )
        .map(|_| ())
    }

    pub fn rename<T: Deref<Target = str>>(
        &mut self,
        selector: &[T],
        new_name: String,
    ) -> Result<(), MutationError> {
        let len = selector.len();
        if len == 0 {
            return Err(IndexingError::NotIndexable.into());
        }

        self.mutate(
            Selector::new(&selector[..len - 1]),
            NodeMutation::Rename {
                before: selector[len - 1].deref(),
                after: new_name,
            },
        )
        .map(|_| ())
    }

    pub fn as_index(&self) -> Index {
        let meta = self.as_meta();
        let kind = match &self.data {
            Kind::Array(nodes) => IndexKind::Array(nodes.len()),
            Kind::Object(index_map) => IndexKind::Object(index_map.keys().cloned().collect()),
            Kind::Null | Kind::Bool(_) | Kind::Number(_) | Kind::String(_) => IndexKind::Terminal,
        };
        Index { meta, kind }
    }

    fn as_meta(&self) -> NodeMeta {
        NodeMeta {
            n_lines: self.n_lines,
            n_bytes: self.n_bytes,
            kind: self.data.node_kind(),
        }
    }
}

impl Node {
    pub fn null() -> Self {
        Self {
            n_lines: 1,
            n_bytes: 4,
            data: Kind::Null,
        }
    }

    fn bool(value: bool) -> Self {
        Self {
            n_lines: 1,
            n_bytes: if value { 4 } else { 5 },
            data: Kind::Bool(value),
        }
    }

    fn number(value: serde_json::Number) -> Result<Self, DeserializationError> {
        let n_bytes = serde_json::to_vec(&value).unwrap().len();
        let data = value
            .as_i64()
            .map(Number::Int)
            .or_else(|| value.as_f64().map(Number::Float))
            .ok_or(DeserializationError::InvalidNumber(value))?;
        Ok(Self {
            n_lines: 1,
            n_bytes,
            data: Kind::Number(data),
        })
    }

    fn string(value: String) -> Self {
        Self {
            n_lines: 1,
            n_bytes: value.len() + 2,
            data: Kind::String(value),
        }
    }

    fn array(values: Vec<serde_json::Value>) -> Result<Self, DeserializationError> {
        if values.is_empty() {
            return Ok(Self {
                n_lines: 1,
                n_bytes: 2,
                data: Kind::Array(Vec::new()),
            });
        }

        let nodes: Vec<Self> = values
            .into_par_iter()
            .map(Self::from_serde_json)
            .collect::<Result<_, _>>()?;

        Ok(Self {
            n_lines: nodes.par_iter().map(|node| node.n_lines).sum::<usize>() + 2,
            n_bytes: nodes.par_iter().map(Self::indented_n_bytes).sum::<usize>()
                + nodes.len()
                + nodes.len().saturating_sub(1)
                + 3,
            data: Kind::Array(nodes),
        })
    }

    fn object(values: IndexMap<String, serde_json::Value>) -> Result<Self, DeserializationError> {
        if values.is_empty() {
            return Ok(Self {
                n_lines: 1,
                n_bytes: 2,
                data: Kind::Object(IndexMap::new()),
            });
        }

        let nodes: IndexMap<String, Self> = values
            .into_par_iter()
            .map(|(key, value)| Ok((key, Self::from_serde_json(value)?)))
            .collect::<Result<_, _>>()?;
        Ok(Self {
            n_lines: nodes.par_values().map(|node| node.n_lines).sum::<usize>() + 2,
            n_bytes: nodes
                .par_iter()
                .map(|(key, node)| 4 + key.len() + node.indented_n_bytes())
                .sum::<usize>()
                + nodes.len()
                + nodes.len().saturating_sub(1)
                + 3,
            data: Kind::Object(nodes),
        })
    }

    fn indented_n_bytes(&self) -> usize {
        self.n_bytes + INDENT * self.n_lines
    }

    fn metas_inner<T: Deref<Target = str>>(
        &self,
        mut selector: Selector<'_, T>,
        metas: &mut Vec<NodeMeta>,
    ) -> Result<(), IndexingError> {
        metas.push(self.as_meta());

        if let Some(next_key) = selector.next() {
            let missing_key = || IndexingError::MissingKey(next_key.to_string());
            let next_node = match &self.data {
                Kind::Array(nodes) => {
                    let index = next_key.parse::<usize>().map_err(|_| missing_key())?;
                    nodes.get(index).ok_or_else(missing_key)?
                }
                Kind::Object(index_map) => index_map.get(next_key).ok_or_else(missing_key)?,
                Kind::Null | Kind::Bool(_) | Kind::Number(_) | Kind::String(_) => {
                    return Err(IndexingError::NotIndexable);
                }
            };

            next_node.metas_inner(selector, metas)
        } else {
            Ok(())
        }
    }

    fn subtree_inner<T: Deref<Target = str>>(
        &self,
        mut selector: Selector<'_, T>,
    ) -> Result<&Self, IndexingError> {
        if let Some(next_key) = selector.next() {
            let missing_key = || IndexingError::MissingKey(next_key.to_string());
            let next_node = match &self.data {
                Kind::Array(nodes) => {
                    let index = next_key.parse::<usize>().map_err(|_| missing_key())?;
                    nodes.get(index).ok_or_else(missing_key)?
                }
                Kind::Object(index_map) => index_map.get(next_key).ok_or_else(missing_key)?,
                Kind::Null | Kind::Bool(_) | Kind::Number(_) | Kind::String(_) => {
                    return Err(IndexingError::NotIndexable);
                }
            };

            next_node.subtree_inner(selector)
        } else {
            Ok(self)
        }
    }

    fn mutate<T: Deref<Target = str>>(
        &mut self,
        mut selector: Selector<'_, T>,
        mutation: NodeMutation,
    ) -> Result<Option<Self>, MutationError> {
        if let Some(next_key) = selector.next() {
            let missing_key = || IndexingError::MissingKey(next_key.to_string());
            let next_node = match &mut self.data {
                Kind::Array(nodes) => {
                    let index = next_key.parse::<usize>().map_err(|_| missing_key())?;
                    nodes.get_mut(index).ok_or_else(missing_key)?
                }
                Kind::Object(index_map) => index_map.get_mut(next_key).ok_or_else(missing_key)?,
                Kind::Null | Kind::Bool(_) | Kind::Number(_) | Kind::String(_) => {
                    return Err(IndexingError::NotIndexable.into());
                }
            };

            let old_n_lines = next_node.n_lines;
            let old_n_bytes = next_node.indented_n_bytes();
            let old_node = next_node.mutate(selector, mutation)?;

            self.n_lines = self.n_lines - old_n_lines + next_node.n_lines;
            self.n_bytes = self.n_bytes - old_n_bytes + next_node.indented_n_bytes();

            Ok(old_node)
        } else {
            match mutation {
                NodeMutation::Replace(mut new_node) => {
                    std::mem::swap(self, &mut new_node);
                    Ok(Some(new_node))
                }
                NodeMutation::Append {
                    after,
                    key: AddNodeKey::Array,
                    node,
                } => match &mut self.data {
                    Kind::Array(child) => {
                        let index = after
                            .parse::<usize>()
                            .map_err(|_| IndexingError::MissingKey(after.to_string()))?;
                        if child.is_empty() {
                            self.n_lines = 2 + node.n_lines;
                            self.n_bytes = 3 + node.indented_n_bytes();
                        } else {
                            self.n_lines += node.n_lines;
                            self.n_bytes += node.indented_n_bytes() + 2;
                        }
                        child.insert(index + 1, node);
                        Ok(None)
                    }
                    Kind::Object(_)
                    | Kind::Null
                    | Kind::Bool(_)
                    | Kind::Number(_)
                    | Kind::String(_) => Err(IndexingError::NotIndexable.into()),
                },
                NodeMutation::Append {
                    after,
                    key: AddNodeKey::Object(new_key),
                    node,
                } => match &mut self.data {
                    Kind::Object(index_map) => {
                        if index_map.contains_key(&new_key) {
                            return Err(MutationError::DuplicateKey);
                        }
                        let Some(index) = index_map.get_index_of(after) else {
                            return Err(IndexingError::MissingKey(after.to_string()).into());
                        };
                        if index_map.is_empty() {
                            self.n_lines = 2 + node.n_lines;
                            self.n_bytes = 7 + new_key.len() + node.indented_n_bytes();
                        } else {
                            self.n_lines += node.n_lines;
                            self.n_bytes += node.indented_n_bytes() + new_key.len() + 6;
                        }
                        index_map.insert_before(index + 1, new_key, node);
                        Ok(None)
                    }
                    Kind::Array(_)
                    | Kind::Null
                    | Kind::Bool(_)
                    | Kind::Number(_)
                    | Kind::String(_) => Err(IndexingError::NotIndexable.into()),
                },
                NodeMutation::Delete(key) => match &mut self.data {
                    Kind::Array(child) => {
                        let index = key
                            .parse::<usize>()
                            .map_err(|_| IndexingError::MissingKey(key.to_string()))?;
                        let deleted_node = child.remove(index);
                        if child.is_empty() {
                            self.n_lines = 1;
                            self.n_bytes = 2;
                        } else {
                            self.n_lines -= deleted_node.n_lines;
                            self.n_bytes -= deleted_node.indented_n_bytes() + 2;
                        }
                        Ok(Some(deleted_node))
                    }
                    Kind::Object(index_map) => {
                        let deleted_node = index_map
                            .shift_remove(key)
                            .ok_or_else(|| IndexingError::MissingKey(key.to_string()))?;
                        if index_map.is_empty() {
                            self.n_lines = 1;
                            self.n_bytes = 2;
                        } else {
                            self.n_lines -= deleted_node.n_lines;
                            self.n_bytes -= deleted_node.indented_n_bytes() + key.len() + 6;
                        }
                        Ok(Some(deleted_node))
                    }
                    Kind::Null | Kind::Bool(_) | Kind::Number(_) | Kind::String(_) => {
                        Err(IndexingError::NotIndexable.into())
                    }
                },
                NodeMutation::Rename { before, after } => match &mut self.data {
                    Kind::Array(_) => Err(MutationError::NotRenameable),
                    Kind::Object(index_map) => {
                        if index_map.contains_key(&after) {
                            return Err(MutationError::DuplicateKey);
                        };
                        let (index, _, node) = index_map
                            .swap_remove_full(before)
                            .ok_or_else(|| IndexingError::MissingKey(before.to_string()))?;
                        self.n_bytes = self.n_bytes + after.len() - before.len();
                        let (last_index, _) = index_map.insert_full(after, node);
                        index_map.swap_indices(index, last_index);
                        Ok(None)
                    }
                    Kind::Null | Kind::Bool(_) | Kind::Number(_) | Kind::String(_) => {
                        Err(IndexingError::NotIndexable.into())
                    }
                },
            }
        }
    }

    fn from_serde_json(value: serde_json::Value) -> Result<Self, DeserializationError> {
        let res = match value {
            serde_json::Value::Null => Self::null(),
            serde_json::Value::Bool(value) => Self::bool(value),
            serde_json::Value::Number(number) => Self::number(number)?,
            serde_json::Value::String(value) => Self::string(value),
            serde_json::Value::Array(values) => Self::array(values)?,
            serde_json::Value::Object(map) => Self::object(map.into_iter().collect())?,
        };
        Ok(res)
    }
}

impl Serialize for Node {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        self.data.serialize(serializer)
    }
}

impl Serialize for Kind {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        match self {
            Kind::Null => serde_json::Value::Null.serialize(serializer),
            Kind::Bool(value) => value.serialize(serializer),
            Kind::Number(number) => number.serialize(serializer),
            Kind::String(value) => value.serialize(serializer),
            Kind::Array(nodes) => nodes.serialize(serializer),
            Kind::Object(index_map) => index_map.serialize(serializer),
        }
    }
}

impl Serialize for Number {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        match self {
            Number::Int(value) => value.serialize(serializer),
            Number::Float(value) => value.serialize(serializer),
        }
    }
}

#[cfg(test)]
const RAW_JSON: &str = r#"{
  "string": "something",
  "int": 123,
  "float": 100.3,
  "bool": true,
  "other_bool": false,
  "null": null,
  "array": [
    1,
    2,
    3.0
  ],
  "nested_object": {
    "key": "value"
  }
}"#;

#[cfg(test)]
mod test {
    use super::*;
    use serde_json::json;

    impl Node {
        fn assert_meta(&self) {
            assert_eq!(
                self.to_string_pretty()
                    .unwrap()
                    .lines()
                    .collect::<Vec<_>>()
                    .len(),
                self.n_lines
            );
            assert_eq!(self.to_string_pretty().unwrap().len(), self.n_bytes);
        }

        fn assert_all_meta(&self) {
            self.assert_meta();
            match &self.data {
                Kind::Array(nodes) => nodes.iter().for_each(Self::assert_meta),
                Kind::Object(index_map) => index_map.values().for_each(Self::assert_meta),
                Kind::Null | Kind::Bool(_) | Kind::Number(_) | Kind::String(_) => {}
            }
        }
    }

    #[test]
    fn round_tripe_test() {
        let res = Node::load(RAW_JSON.as_bytes())
            .unwrap()
            .to_string_pretty()
            .unwrap();
        assert_eq!(res, RAW_JSON);
    }

    #[test]
    fn strip_insignificant_whitespace_drops_padding_outside_strings() {
        let input = b"{\n  \"a\" :\t1,\r\n  \"b\": [ 1,\n2 ]\n}\n";
        let stripped = strip_insignificant_whitespace(&input[..]).unwrap();
        assert_eq!(stripped, br#"{"a":1,"b":[1,2]}"#);
    }

    #[test]
    fn strip_insignificant_whitespace_preserves_whitespace_inside_strings() {
        let input = br#"{ "key" : "hello world\ttab" }"#;
        let stripped = strip_insignificant_whitespace(&input[..]).unwrap();
        assert_eq!(stripped, br#"{"key":"hello world\ttab"}"#);
    }

    #[test]
    fn strip_insignificant_whitespace_handles_escaped_quotes() {
        let input = br#"{ "key" : "say \"hi\" to \\" }"#;
        let stripped = strip_insignificant_whitespace(&input[..]).unwrap();
        assert_eq!(stripped, br#"{"key":"say \"hi\" to \\"}"#);
    }

    #[test]
    fn load_with_extra_whitespace_matches_compact_load() {
        let padded = "{\n  \"a\"  :   [1,   2,\t3],\n  \"b\" : \"has space\"\n}\n";
        let compact = r#"{"a":[1,2,3],"b":"has space"}"#;

        let from_padded = Node::load(padded.as_bytes()).unwrap();
        let from_compact = Node::load(compact.as_bytes()).unwrap();

        assert_eq!(from_padded, from_compact);
    }

    #[test]
    fn json_value_test() {
        let json_value = json!({
            "string": "something",
            "int": 123,
            "float": 100.3,
            "bool": true,
            "other_bool": false,
            "null": null,
            "array": [1, 2, 3.],
            "nested_object": {
                "key": "value"
            }
        });

        let from_node = Node::from_serde_json(json_value.clone()).unwrap();
        assert_eq!(
            sonic_rs::to_string(&from_node).unwrap(),
            sonic_rs::to_string(&json_value).unwrap(),
        );
    }

    #[test]
    fn node_meta_test() {
        let json_value = json!({
            "string": "something",
            "int": 123,
            "float": 100.3,
            "bool": true,
            "other_bool": false,
            "null": null,
            "array": [
                1,
                2,
                3.
            ],
            "nested_object": {
                "key": "value"
            }
        });

        let node = Node::from_serde_json(json_value.clone()).unwrap();
        node.assert_all_meta();

        let Kind::Object(fields) = node.data else {
            unreachable!()
        };

        assert_eq!(
            fields.keys().collect::<Vec<_>>(),
            [
                "string",
                "int",
                "float",
                "bool",
                "other_bool",
                "null",
                "array",
                "nested_object",
            ]
        );
    }

    #[test]
    fn empty_node_meta_test() {
        for json_value in [
            json!([]),
            json!({}),
            json!(null),
            json!([null]),
            json!(""),
            json!([""]),
        ] {
            let node = Node::from_serde_json(json_value).unwrap();
            node.assert_all_meta();
        }
    }

    #[test]
    fn keys_test() {
        let node = Node::load(RAW_JSON.as_bytes()).unwrap();
        assert_eq!(
            node.subtree::<&str>(&[]).unwrap().as_index(),
            Index {
                meta: NodeMeta {
                    n_lines: 16,
                    n_bytes: 199,
                    kind: NodeKind::Object,
                },
                kind: IndexKind::Object(vec![
                    String::from("string"),
                    String::from("int"),
                    String::from("float"),
                    String::from("bool"),
                    String::from("other_bool"),
                    String::from("null"),
                    String::from("array"),
                    String::from("nested_object"),
                ])
            }
        );

        assert_eq!(
            node.subtree(&["array"]).unwrap().as_index(),
            Index {
                meta: NodeMeta {
                    n_lines: 5,
                    n_bytes: 19,
                    kind: NodeKind::Array,
                },
                kind: IndexKind::Array(3)
            }
        );
        assert_eq!(
            node.subtree(&["array", "0"]).unwrap().as_index(),
            Index {
                meta: NodeMeta {
                    n_lines: 1,
                    n_bytes: 1,
                    kind: NodeKind::Terminal,
                },
                kind: IndexKind::Terminal
            }
        );
        assert_eq!(
            node.subtree(&["nested_object"]).unwrap().as_index(),
            Index {
                meta: NodeMeta {
                    n_lines: 3,
                    n_bytes: 20,
                    kind: NodeKind::Object,
                },
                kind: IndexKind::Object(vec![String::from("key")])
            }
        );
        assert_eq!(
            node.subtree(&["nested_object", "key"]).unwrap().as_index(),
            Index {
                meta: NodeMeta {
                    n_lines: 1,
                    n_bytes: 7,
                    kind: NodeKind::Terminal,
                },
                kind: IndexKind::Terminal
            }
        );

        assert_eq!(
            node.subtree(&["int"]).unwrap().as_index(),
            Index {
                meta: NodeMeta {
                    n_lines: 1,
                    n_bytes: 3,
                    kind: NodeKind::Terminal,
                },
                kind: IndexKind::Terminal
            }
        );
        assert_eq!(
            node.subtree(&["int", "2"]).unwrap_err(),
            IndexingError::NotIndexable
        );
        assert_eq!(
            node.subtree(&["nested_object", "not_found"]).unwrap_err(),
            IndexingError::MissingKey(String::from("not_found"))
        );
    }

    #[test]
    fn replace_test() {
        let original = json!({
            "a": "x",
            "b": "x",
            "nested": {
                "key": "value"
            },
            "array": [
                1,
                2,
                3
            ]
        });

        let mut node = Node::from_serde_json(original).unwrap();
        let new_node = Node::from_serde_json(json!(["cat", "dog"])).unwrap();
        let replaced_node = node.replace(&["nested", "key"], new_node).unwrap();

        assert_eq!(
            replaced_node,
            Node::from_serde_json(json!("value")).unwrap()
        );
        assert_eq!(
            node,
            Node::from_serde_json(json!({
                "a": "x",
                "b": "x",
                "nested": {
                    "key": [
                        "cat",
                        "dog"
                    ]
                },
                "array": [
                    1,
                    2,
                    3
                ]
            }))
            .unwrap()
        );

        node.assert_all_meta();
    }

    #[test]
    fn rename_test() {
        let original = json!({
            "a": "x",
            "b": "x",
            "nested": {
                "key": "value",
                "other_key": "other_value",
                "tail": "tail_value"
            },
            "array": [
                1,
                2,
                3
            ]
        });

        let mut node = Node::from_serde_json(original).unwrap();
        node.rename(&["nested", "other_key"], String::from("new_key"))
            .unwrap();

        assert_eq!(
            node,
            Node::from_serde_json(json!({
                "a": "x",
                "b": "x",
                "nested": {
                    "key": "value",
                    "new_key": "other_value",
                    "tail": "tail_value"
                },
                "array": [
                    1,
                    2,
                    3
                ]
            }))
            .unwrap()
        );

        node.assert_all_meta();
    }

    #[test]
    fn delete_from_array_test() {
        let original = json!({
            "array": [
                1,
                2,
                3
            ]
        });

        let mut node = Node::from_serde_json(original).unwrap();
        node.delete(&["array", "0"]).unwrap();

        assert_eq!(
            node,
            Node::from_serde_json(json!({
                "array": [
                    2,
                    3
                ]
            }))
            .unwrap()
        );

        node.assert_all_meta();
    }

    #[test]
    fn delete_from_array_last_test() {
        let original = json!({
            "array": [
                1,
                2,
                3
            ]
        });

        let mut node = Node::from_serde_json(original).unwrap();
        node.delete(&["array", "2"]).unwrap();

        assert_eq!(
            node,
            Node::from_serde_json(json!({
                "array": [
                    1,
                    2
                ]
            }))
            .unwrap()
        );

        node.assert_all_meta();
    }

    #[test]
    fn delete_from_array_empty_test() {
        let original = json!({
            "array": [
                1,
                2,
                3
            ]
        });

        let mut node = Node::from_serde_json(original).unwrap();
        for _ in 0..3 {
            node.delete(&["array", "0"]).unwrap();
        }

        assert_eq!(
            node,
            Node::from_serde_json(json!({
                "array": []
            }))
            .unwrap()
        );

        node.assert_all_meta();
    }

    #[test]
    fn delete_from_array_object_test() {
        let original = json!({
            "array": [
                1,
                {"key": "value"},
                3
            ]
        });

        let mut node = Node::from_serde_json(original).unwrap();
        node.delete(&["array", "1"]).unwrap();

        assert_eq!(
            node,
            Node::from_serde_json(json!({
                "array": [1, 3]
            }))
            .unwrap()
        );

        node.assert_all_meta();
    }

    #[test]
    fn delete_from_object_test() {
        let original = json!({
            "key": "1",
            "other_key": "2",
            "new_key": {
                "nested": "value"
            }
        });

        let mut node = Node::from_serde_json(original).unwrap();
        node.delete(&["key"]).unwrap();

        assert_eq!(
            node,
            Node::from_serde_json(json!({
                "other_key": "2",
                "new_key": {
                    "nested": "value"
                }
            }))
            .unwrap()
        );

        node.assert_all_meta();
    }

    #[test]
    fn delete_from_object_nested_test() {
        let original = json!({
            "key": "1",
            "other_key": "2",
            "new_key": {
                "nested": "value"
            }
        });

        let mut node = Node::from_serde_json(original).unwrap();
        node.delete(&["new_key"]).unwrap();

        assert_eq!(
            node,
            Node::from_serde_json(json!({
            "key": "1",
                "other_key": "2"
            }))
            .unwrap()
        );

        node.assert_all_meta();
    }

    #[test]
    fn delete_from_object_empty_test() {
        let original = json!({
            "key": "1",
            "other_key": "2",
            "new_key": {
                "nested": "value"
            }
        });

        let mut node = Node::from_serde_json(original).unwrap();
        node.delete(&["new_key"]).unwrap();
        node.delete(&["key"]).unwrap();
        node.delete(&["other_key"]).unwrap();

        assert_eq!(node, Node::from_serde_json(json!({})).unwrap());

        node.assert_all_meta();
    }

    #[test]
    fn append_after_into_object() {
        let original = json!({
            "key": "1",
            "other_key": "2",
            "new_key": {
                "nested": "value"
            }
        });

        let mut node = Node::from_serde_json(original).unwrap();
        node.append_after(
            &["other_key"],
            AddNodeKey::Object(String::from("k")),
            Node::bool(true),
        )
        .unwrap();
        assert_eq!(
            node.append_after(
                &["new_key"],
                AddNodeKey::Object(String::from("k")),
                Node::bool(true),
            )
            .unwrap_err(),
            MutationError::DuplicateKey
        );

        assert_eq!(
            node,
            Node::from_serde_json(json!({
                "key": "1",
                "other_key": "2",
                "k": true,
                "new_key": {
                    "nested": "value"
                }
            }))
            .unwrap()
        );

        node.assert_all_meta();
    }

    #[test]
    fn append_after_into_array() {
        let original = json!({
            "key": "1",
            "other_key": "2",
            "new_key": [true, "false"]
        });

        let mut node = Node::from_serde_json(original).unwrap();
        node.append_after(&["new_key", "0"], AddNodeKey::Array, Node::null())
            .unwrap();
        node.append_after(&["new_key", "2"], AddNodeKey::Array, Node::null())
            .unwrap();

        assert_eq!(
            node,
            Node::from_serde_json(json!({
                "key": "1",
                "other_key": "2",
                "new_key": [true, null, "false", null]
            }))
            .unwrap()
        );

        node.assert_all_meta();
    }
}
