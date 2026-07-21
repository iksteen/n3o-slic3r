// Mock of @tauri-apps/api/webview for the browser demo. Only the drag-drop
// subscription DropZone uses is stubbed — file drop is inert in the demo.

export function getCurrentWebview() {
  return {
    onDragDropEvent: async (_cb: unknown): Promise<() => void> => () => {},
  };
}
