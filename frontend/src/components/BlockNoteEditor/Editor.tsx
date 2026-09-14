"use client";

import { useEffect } from "react";
import type { PartialBlock, Block } from "@blocknote/core";
import { useCreateBlockNote } from "@blocknote/react";
import { BlockNoteView } from "@blocknote/shadcn";
import "@blocknote/shadcn/style.css";
import "@blocknote/core/fonts/inter.css";
import { useTheme } from "@/contexts/ThemeContext";

interface EditorProps {
  initialContent?: Block[];
  onChange?: (blocks: Block[]) => void;
  editable?: boolean;
}

export default function Editor({ initialContent, onChange, editable = true }: EditorProps) {
  // specs/0057 — BlockNote follows the app theme (Faceplate/Deck) instead of pinning light.
  const { resolved } = useTheme();

  console.log('📝 EDITOR: Initializing BlockNote editor with blocks:', {
    hasContent: !!initialContent,
    blocksCount: initialContent?.length || 0,
    editable
  });

  const editor = useCreateBlockNote({
    // BlockNote rejects an empty array ("initialContent must be a non-empty array of blocks");
    // pass undefined for a blank editor (e.g. a fresh notepad with no saved notes).
    initialContent:
      initialContent && initialContent.length > 0
        ? (initialContent as PartialBlock[])
        : undefined,
  });

  console.log('📝 EDITOR: BlockNote editor created successfully');

  // Handle content changes
  useEffect(() => {
    if (!onChange) return;

    const handleChange = () => {
      console.log('📝 EDITOR: Content changed, notifying parent...', {
        blocksCount: editor.document.length
      });
      onChange(editor.document);
    };

    const unsubscribe = editor.onChange(handleChange);

    return () => {
      if (typeof unsubscribe === 'function') {
        console.log('📝 EDITOR: Cleaning up onChange listener');
        unsubscribe();
      }
    };
  }, [editor, onChange]);

  // Keep the editor simple: no slash menu, drag-handle side menu, or emoji/file
  // pickers — just an editable surface with the contextual formatting toolbar.
  return (
    <BlockNoteView
      editor={editor}
      editable={editable}
      theme={resolved}
      sideMenu={false}
      slashMenu={false}
      emojiPicker={false}
      filePanel={false}
    />
  );
}
