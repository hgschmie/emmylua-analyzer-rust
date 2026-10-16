//! # flow -- per-file control flow graph (CFG)
//!
//! Ported from the old `compilation/analyzer/flow` system; the decl model is now `SemanticId`.
//! `FlowTree` is the per-file fact layer (like `file_facts`): it is invalidated automatically on file changes.
//! Used by later flow-sensitive type queries (narrowing / assignment flow).

mod bind_binary_expr;
mod binder;
mod comment;
mod engine;
mod exprs;
mod flow_node;
mod flow_tree;
mod stats;

use emmylua_parser::LuaChunk;

use crate::FileId;

use super::SemanticDatabase;
use super::query::file_facts;

pub use binder::FlowBinder;
pub use flow_node::*;
pub use flow_tree::*;

/// Merges a label's incoming flow (folds back predecessors), matching the old `bind_analyze::finish_flow_label`.
fn finish_flow_label(binder: &mut FlowBinder, label: FlowId, default: FlowId) -> FlowId {
    if let Some(flow_node) = binder.get_flow(label) {
        let Some(antecedent) = &flow_node.antecedent else {
            // With no reachable entry there is no loop transfer to preserve.
            return default;
        };
        // Even a single reachable entry carries loop mutation information.
        if flow_node.kind.is_loop_label() {
            return label;
        }
        if let FlowAntecedent::Single(existing_id) = antecedent {
            return *existing_id;
        }
    } else {
        return binder.unreachable;
    }
    label
}

/// Per-file control flow graph. Pure lookup in the write-time built `SemanticDatabase::flow_trees`.
pub(crate) fn flow_tree_of(db: &SemanticDatabase, file: FileId) -> &FlowTree {
    db.flow_tree_of(file)
}

pub(super) fn build_flow_tree(db: &SemanticDatabase, file: FileId) -> FlowTree {
    let file_id = file;
    let facts = file_facts(db, file);
    let tree = db
        .vfs()
        .get_syntax_tree(&file)
        .expect("syntax tree must exist");
    let chunk: LuaChunk = tree.get_chunk_node();
    let mut binder = FlowBinder::new(file_id, facts);
    let start = binder.start;
    if let Some(block) = chunk.get_block() {
        engine::run_bind_block(&mut binder, block, start);
    }
    binder.finish()
}

#[cfg(test)]
mod loop_label_tests {
    use super::*;
    use emmylua_parser::{LuaAstNode, LuaWhileStat};

    #[test]
    fn loop_labels_preserve_entry_order_and_empty_fallback() {
        let mut ws = crate::VirtualWorkspace::new();
        let file = ws.def("while flag do end");
        let model = ws.analysis.semantic_model(file);
        let syntax = model
            .chunk()
            .unwrap()
            .descendants::<LuaWhileStat>()
            .next()
            .unwrap()
            .get_syntax_id();
        let mut binder = FlowBinder::new(file, model.file_facts().unwrap());
        let label = binder.create_loop_label(syntax);
        let entry = binder.start;
        let unreachable = binder.unreachable;
        binder.add_antecedent(label, unreachable);
        assert!(binder.get_flow(label).unwrap().antecedent.is_none());
        assert_eq!(
            finish_flow_label(&mut binder, label, unreachable),
            unreachable
        );
        assert_eq!(finish_flow_label(&mut binder, label, entry), entry);
        binder.add_antecedent(label, entry);
        assert_eq!(finish_flow_label(&mut binder, label, entry), label);
        let continued = binder.create_continue();
        binder.add_antecedent(label, continued);
        assert_eq!(finish_flow_label(&mut binder, label, entry), label);
        let tree = binder.finish();
        let Some(FlowAntecedent::Multiple(join)) = tree.get_flow_node(label).unwrap().antecedent
        else {
            panic!("the loop must retain its entry and continue predecessors");
        };
        assert_eq!(
            tree.get_multi_antecedents(join).unwrap(),
            &[entry, continued]
        );
    }
}
