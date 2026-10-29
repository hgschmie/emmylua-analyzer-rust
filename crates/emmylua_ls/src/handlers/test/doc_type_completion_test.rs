#[cfg(test)]
mod tests {
    use emmylua_code_analysis::WorkspaceFolder;
    use googletest::prelude::*;
    use lsp_types::{
        CompletionItem, CompletionItemKind, CompletionResponse, CompletionTextEdit,
        CompletionTriggerKind,
    };
    use tokio_util::sync::CancellationToken;

    use crate::handlers::{completion::completion, test_lib::ProviderVirtualWorkspace};

    fn items(ws: &mut ProviderVirtualWorkspace, text: &str) -> Result<Vec<CompletionItem>> {
        let (content, position) = ProviderVirtualWorkspace::handle_file_content(text)?;
        let file = ws.def(&content);
        let response = completion(
            &ws.analysis,
            file,
            position,
            CompletionTriggerKind::INVOKED,
            CancellationToken::new(),
        )
        .or_fail()?;
        Ok(match response {
            CompletionResponse::Array(items) => items,
            CompletionResponse::List(list) => list.items,
        })
    }

    #[gtest]
    fn doc_types_include_library_metadata_and_standard_types() -> Result<()> {
        let mut ws = ProviderVirtualWorkspace::new_with_init_std_lib();
        ws.analysis.add_library_workspace(&WorkspaceFolder::new(
            ws.virtual_url_generator.new_path("library"),
            true,
        ));
        ws.def_file(
            "library/types.lua",
            "---@meta _\n---@class CompletionLibrary\nlocal Library = {}\n",
        );
        ws.def_file(
            "main-types.lua",
            "---@meta\n---@enum CompletionEnum\nlocal Enum = { ON = 1 }\n",
        );
        let result = items(&mut ws, "---@type Completion<??>\nlocal value")?;
        for (name, kind) in [
            ("CompletionLibrary", CompletionItemKind::CLASS),
            ("CompletionEnum", CompletionItemKind::ENUM),
        ] {
            let item = result.iter().find(|item| item.label == name).or_fail()?;
            verify_eq!(item.kind, Some(kind))?;
        }
        let result = items(&mut ws, "---@type iolib.Op<??>\nlocal value")?;
        let standard = result
            .iter()
            .find(|item| item.label == "OpenMode")
            .or_fail()?;
        verify_eq!(standard.kind, Some(CompletionItemKind::STRUCT))?;
        Ok(())
    }

    #[gtest]
    fn doc_types_match_qualified_prefix_and_replace_the_complete_token() -> Result<()> {
        let mut ws = ProviderVirtualWorkspace::new();
        ws.def_file("types.lua", "---@namespace data\n---@class LampPrototype\nlocal Lamp = {}\n---@class OtherPrototype\nlocal Other = {}\n");
        ws.def_file(
            "unrelated.lua",
            "---@class data2.LampPrototype\nlocal Lamp = {}\n",
        );
        for prefix in [
            "data.L",
            "data.Lamp",
            "data.La<??>mpWrong",
            "data.<??>OldName",
        ] {
            let annotated = if prefix.contains("<??>") {
                prefix.to_owned()
            } else {
                format!("{prefix}<??>")
            };
            let text = format!("---@type {annotated}\nlocal value");
            let result = items(&mut ws, &text)?;
            let item = result
                .iter()
                .find(|item| item.label == "LampPrototype")
                .or_fail()?;
            verify_eq!(item.filter_text.as_deref(), Some("data.LampPrototype"))?;
            let Some(CompletionTextEdit::Edit(edit)) = &item.text_edit else {
                panic!("missing type edit: {item:?}")
            };
            let original_line = text.lines().next().unwrap().replace("<??>", "");
            verify_eq!(edit.range.start.line, 0)?;
            verify_eq!(edit.range.end.line, 0)?;
            let mut edited = original_line;
            edited.replace_range(
                edit.range.start.character as usize..edit.range.end.character as usize,
                &edit.new_text,
            );
            verify_eq!(edited, "---@type data.LampPrototype")?;
            if !prefix.starts_with("data.<??>") {
                verify_eq!(result.len(), 1)?;
            }
        }
        Ok(())
    }

    #[gtest]
    fn doc_types_preserve_visibility_and_deduplicate_partial_classes() -> Result<()> {
        let mut ws = ProviderVirtualWorkspace::new();
        ws.analysis.add_library_workspace(&WorkspaceFolder::new(
            ws.virtual_url_generator.new_path("library"),
            true,
        ));
        ws.def_file("library/types.lua", "---@class (public) VisiblePublic\nlocal P = {}\n---@class (internal) VisibleInternal\nlocal I = {}\n---@class (file) VisiblePrivate\nlocal F = {}\n");
        ws.def_file("other.lua", "---@class (file) VisibleOtherPrivate\nlocal P = {}\n---@class (partial) VisiblePartial\nlocal Partial = {}\n");
        ws.def_file(
            "partial.lua",
            "---@class (partial) VisiblePartial\nlocal Partial = {}\n",
        );
        let result = items(
            &mut ws,
            "---@class (file) VisibleOwnPrivate\nlocal Own = {}\n---@type Visible<??>\nlocal value",
        )?;
        let mut labels = result
            .iter()
            .map(|item| item.label.as_str())
            .collect::<Vec<_>>();
        labels.sort();
        verify_eq!(
            labels,
            vec!["VisibleOwnPrivate", "VisiblePartial", "VisiblePublic"]
        )?;
        ws.virtual_url_generator.base = ws.virtual_url_generator.new_path("library");
        let result = items(&mut ws, "---@type Visible<??>\nlocal value")?;
        verify_eq!(
            result
                .iter()
                .filter(|item| item.label == "VisibleInternal")
                .count(),
            1
        )?;
        verify_eq!(
            result
                .iter()
                .filter(|item| item.label == "VisiblePrivate")
                .count(),
            0
        )?;
        Ok(())
    }

    #[gtest]
    fn doc_types_insert_at_empty_annotation_positions() -> Result<()> {
        let mut ws = ProviderVirtualWorkspace::new();
        ws.def_file("types.lua", "---@class CompletionValue\nlocal Value = {}\n");
        for annotation in ["---@type ", "---@return ", "---@param value "] {
            let result = items(&mut ws, &format!("{annotation}<??>\nlocal value"))?;
            let item = result
                .iter()
                .find(|item| item.label == "CompletionValue")
                .or_fail()?;
            let Some(CompletionTextEdit::Edit(edit)) = &item.text_edit else {
                panic!("missing type edit")
            };
            verify_eq!(edit.range.start, edit.range.end)?;
            verify_eq!(edit.range.start.character as usize, annotation.len())?;
            verify_eq!(edit.new_text.as_str(), "CompletionValue")?;
        }
        Ok(())
    }

    fn apply_first_line_edit(text: &str, item: &CompletionItem) -> Result<String> {
        let Some(CompletionTextEdit::Edit(edit)) = &item.text_edit else {
            panic!("missing type edit: {item:?}");
        };
        verify_eq!(edit.range.start.line, 0)?;
        verify_eq!(edit.range.end.line, 0)?;
        let mut line = text
            .lines()
            .next()
            .unwrap()
            .replace("<??>", "")
            .encode_utf16()
            .collect::<Vec<_>>();
        line.splice(
            edit.range.start.character as usize..edit.range.end.character as usize,
            edit.new_text.encode_utf16(),
        );
        String::from_utf16(&line).or_fail()
    }

    #[gtest]
    fn doc_types_replace_multibyte_prefixes_with_character_ranges() -> Result<()> {
        let mut ws = ProviderVirtualWorkspace::new();
        ws.def_file("types.lua", "---@class 数据.灯Prototype\nlocal Lamp = {}\n");
        let text = "---@type 数据.灯<??>旧\nlocal value";
        let result = items(&mut ws, text)?;
        let item = result
            .iter()
            .find(|item| item.label == "灯Prototype")
            .or_fail()?;
        verify_eq!(item.filter_text.as_deref(), Some("数据.灯Prototype"))?;
        verify_eq!(
            apply_first_line_edit(text, item)?,
            "---@type 数据.灯Prototype"
        )?;
        Ok(())
    }

    #[gtest]
    fn doc_attribute_types_keep_class_filter_and_qualified_replacement() -> Result<()> {
        let mut ws = ProviderVirtualWorkspace::new();
        ws.def_file(
            "types.lua",
            "---@class attrs.Constructor\nlocal C = {}\n---@alias attrs.Constant integer\n---@enum attrs.Color\nlocal Color = { RED = 1 }\n",
        );
        let text = "---@[attrs.Co<??>Old]\nlocal value";
        let result = items(&mut ws, text)?;
        verify_eq!(result.len(), 1)?;
        let item = &result[0];
        verify_eq!(item.label.as_str(), "Constructor")?;
        verify_eq!(item.kind, Some(CompletionItemKind::CLASS))?;
        verify_eq!(item.filter_text.as_deref(), Some("attrs.Constructor"))?;
        verify_eq!(
            apply_first_line_edit(text, item)?,
            "---@[attrs.Constructor]"
        )?;
        Ok(())
    }

    #[gtest]
    fn doc_types_replace_existing_name_at_its_start() -> Result<()> {
        let mut ws = ProviderVirtualWorkspace::new();
        ws.def_file("types.lua", "---@class CompletionValue\nlocal Value = {}\n");
        let text = "---@type <??>OldName\nlocal value";
        let result = items(&mut ws, text)?;
        let item = result
            .iter()
            .find(|item| item.label == "CompletionValue")
            .or_fail()?;
        verify_eq!(
            apply_first_line_edit(text, item)?,
            "---@type CompletionValue"
        )?;
        Ok(())
    }

    #[gtest]
    fn doc_attributes_replace_existing_name_after_the_opener() -> Result<()> {
        let mut ws = ProviderVirtualWorkspace::new();
        ws.def_file(
            "types.lua",
            "---@class CompletionAttribute\nlocal Attribute = {}\n",
        );
        let text = "---@[<??>OldName]\nlocal value";
        let result = items(&mut ws, text)?;
        let item = result
            .iter()
            .find(|item| item.label == "CompletionAttribute")
            .or_fail()?;
        verify_eq!(
            apply_first_line_edit(text, item)?,
            "---@[CompletionAttribute]"
        )?;
        Ok(())
    }

    #[gtest]
    fn doc_types_include_forward_declarations_in_the_same_file() -> Result<()> {
        let mut ws = ProviderVirtualWorkspace::new();
        let text = "---@type Later<??>\nlocal value\n---@class Later\nlocal Later = {}";
        let result = items(&mut ws, text)?;
        let item = result.iter().find(|item| item.label == "Later").or_fail()?;
        verify_eq!(item.filter_text.as_deref(), Some("Later"))?;
        verify_eq!(apply_first_line_edit(text, item)?, "---@type Later")?;
        Ok(())
    }
}
