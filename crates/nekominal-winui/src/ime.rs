//! `Windows.UI.Text.Core.CoreTextEditContext` as the IME endpoint of the terminal host.
//!
//! Seed of nekominal-winui::ime (from chrome-winui-src core_text.rs, decoupled from the chrome: the host
//! supplies the cursor-cell screen rectangle and receives commits and preedit changes as callbacks).
//!
//! The document only ever holds the preedit. TSF positions are UTF-16 caret positions, so the document is
//! kept as UTF-16 and converted when read. `CompositionCompleted` carries the commit: the document at that
//! moment is the committed text; it is handed to `on_commit` and the document is emptied.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use nekominal_winui_sys::*;
use windows_core::{EventRevoker, Result};

use crate::diag::log;

/// Screen rectangle (physical pixels) of the terminal cursor cell and of the whole pane.
pub struct Layout {
    pub cursor_cell: (f32, f32, f32, f32),
    pub pane: (f32, f32, f32, f32),
}

struct State {
    text: RefCell<Vec<u16>>,
    selection: Cell<(i32, i32)>,
    composing: Cell<bool>,
    layout: Box<dyn Fn() -> Option<Layout>>,
    on_commit: Box<dyn Fn(String)>,
}

pub struct ImeHost {
    ctx: CoreTextEditContext,
    state: Rc<State>,
    _revokers: Vec<EventRevoker>,
}

fn slice(text: &[u16], r: CoreTextRange) -> String {
    let len = text.len() as i32;
    let (s, e) = (r.start_caret_position.clamp(0, len), r.end_caret_position.clamp(0, len));
    String::from_utf16_lossy(&text[s.min(e) as usize..s.max(e) as usize])
}

impl ImeHost {
    pub fn new(layout: impl Fn() -> Option<Layout> + 'static, on_commit: impl Fn(String) + 'static) -> Result<Self> {
        let ctx = CoreTextServicesManager::GetForCurrentView()?.CreateEditContext()?;
        ctx.SetInputScope(CoreTextInputScope::Default)?;
        ctx.SetInputPaneDisplayPolicy(CoreTextInputPaneDisplayPolicy::Manual)?;
        let state = Rc::new(State {
            text: RefCell::new(Vec::new()),
            selection: Cell::new((0, 0)),
            composing: Cell::new(false),
            layout: Box::new(layout),
            on_commit: Box::new(on_commit),
        });
        let mut r = Vec::new();

        let s = state.clone();
        r.push(ctx.TextRequested(move |_, args| {
            let Some(req) = args.as_ref().and_then(|a| a.Request().ok()) else { return };
            let text = slice(&s.text.borrow(), req.Range().unwrap_or_default());
            let _ = req.SetText(&text);
        })?);

        let s = state.clone();
        r.push(ctx.SelectionRequested(move |_, args| {
            let Some(req) = args.as_ref().and_then(|a| a.Request().ok()) else { return };
            let (a, b) = s.selection.get();
            let _ = req.SetSelection(CoreTextRange { start_caret_position: a, end_caret_position: b });
        })?);

        // The candidate window is placed from these bounds: screen coordinates in physical pixels.
        let s = state.clone();
        r.push(ctx.LayoutRequested(move |_, args| {
            let Some(req) = args.as_ref().and_then(|a| a.Request().ok()) else { return };
            let Ok(bounds) = req.LayoutBounds() else { return };
            let Some(layout) = (s.layout)() else { return };
            let rect = |(x, y, width, height): (f32, f32, f32, f32)| Rect { x, y, width, height };
            let _ = bounds.SetTextBounds(rect(layout.cursor_cell));
            let _ = bounds.SetControlBounds(rect(layout.pane));
            log(&format!("IME LayoutRequested -> cursor cell {:?}", layout.cursor_cell));
        })?);

        let s = state.clone();
        r.push(ctx.TextUpdating(move |_, args| {
            let Some(a) = args.as_ref() else { return };
            let range = a.Range().unwrap_or_default();
            let text = a.Text().unwrap_or_default();
            let sel = a.NewSelection().unwrap_or_default();
            let mut doc = s.text.borrow_mut();
            let len = doc.len() as i32;
            let (st, en) = (range.start_caret_position.clamp(0, len) as usize, range.end_caret_position.clamp(0, len) as usize);
            doc.splice(st.min(en)..st.max(en), text.encode_utf16());
            s.selection.set((sel.start_caret_position, sel.end_caret_position));
            log(&format!("IME TextUpdating -> preedit '{}'", String::from_utf16_lossy(&doc)));
            let _ = a.SetResult(CoreTextTextUpdatingResult::Succeeded);
        })?);

        let s = state.clone();
        r.push(ctx.SelectionUpdating(move |_, args| {
            let Some(a) = args.as_ref() else { return };
            let sel = a.Selection().unwrap_or_default();
            s.selection.set((sel.start_caret_position, sel.end_caret_position));
            let _ = a.SetResult(CoreTextSelectionUpdatingResult::Succeeded);
        })?);

        r.push(ctx.FormatUpdating(move |_, args| {
            let Some(a) = args.as_ref() else { return };
            let _ = a.SetResult(CoreTextFormatUpdatingResult::Succeeded);
        })?);

        let s = state.clone();
        r.push(ctx.CompositionStarted(move |_, _| s.composing.set(true))?);

        let s = state.clone();
        r.push(ctx.CompositionCompleted(move |c, _| {
            s.composing.set(false);
            let committed = String::from_utf16_lossy(&s.text.borrow());
            let len = s.text.borrow().len() as i32;
            s.text.borrow_mut().clear();
            s.selection.set((0, 0));
            log(&format!("IME COMMIT '{committed}'"));
            if !committed.is_empty() {
                (s.on_commit)(committed);
            }
            if let Some(c) = c.as_ref() {
                let _ = c.NotifyTextChanged(
                    CoreTextRange { start_caret_position: 0, end_caret_position: len },
                    0,
                    CoreTextRange { start_caret_position: 0, end_caret_position: 0 },
                );
            }
        })?);

        Ok(Self { ctx, state, _revokers: r })
    }

    pub fn focus_enter(&self) {
        let r = self.ctx.NotifyFocusEnter();
        log(&format!("IME NotifyFocusEnter -> {r:?}"));
    }

    pub fn focus_leave(&self) {
        let r = self.ctx.NotifyFocusLeave();
        log(&format!("IME NotifyFocusLeave -> {r:?}"));
    }

    /// The text being composed, `None` outside a composition.
    pub fn preedit(&self) -> Option<String> {
        let doc = self.state.text.borrow();
        (!doc.is_empty()).then(|| String::from_utf16_lossy(&doc))
    }

    pub fn notify_layout_changed(&self) {
        let _ = self.ctx.NotifyLayoutChanged();
    }
}
