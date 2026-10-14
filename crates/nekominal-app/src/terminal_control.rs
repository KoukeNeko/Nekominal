//! A custom terminal control written in Rust: `TerminalControl : UserControl` (WinRT composition / aggregation)
//! whose `OnCreateAutomationPeer` returns `TerminalPeer : FrameworkElementAutomationPeer`, which exposes the
//! UI Automation Text pattern over the visible terminal text (`TextModel`).
//!
//! Everything not overridden is forwarded to the inner (base) object, which the generated `*_Impl` traits
//! require because every method of an overrides interface must be implemented.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use windows_core::{Array, Error, HSTRING, IInspectable, IUnknownImpl, Interface, Ref, Result, implement_decl};

use crate::util::log;
use nekominal_winui_sys::*;

const E_POINTER: windows_core::HRESULT = windows_core::HRESULT(0x80004003_u32 as i32);
const E_NOTIMPL: windows_core::HRESULT = windows_core::HRESULT(0x80004001_u32 as i32);
const E_FAIL: windows_core::HRESULT = windows_core::HRESULT(0x80004005_u32 as i32);

// ---------------------------------------------------------------------------------------------
// Shared text model (what the renderer would feed from the emulator's visible screen)
// ---------------------------------------------------------------------------------------------
#[derive(Default)]
pub struct TextModel {
    /// Visible screen text, lines separated by '\n'.
    pub text: RefCell<String>,
    /// Caret as a (start, end) range in characters (Unicode scalar values).
    pub caret: Cell<(i32, i32)>,
}

impl TextModel {
    pub fn len(&self) -> i32 {
        self.text.borrow().chars().count() as i32
    }
    pub fn slice(&self, start: i32, end: i32) -> String {
        self.text.borrow().chars().skip(start.max(0) as usize).take((end - start).max(0) as usize).collect()
    }
    /// Start offsets of every line.
    fn line_starts(&self) -> Vec<i32> {
        let mut v = vec![0];
        for (i, c) in self.text.borrow().chars().enumerate() {
            if c == '\n' {
                v.push(i as i32 + 1);
            }
        }
        v
    }
    fn line_bounds(&self, pos: i32) -> (i32, i32) {
        let starts = self.line_starts();
        let len = self.len();
        let idx = starts.iter().rposition(|s| *s <= pos).unwrap_or(0);
        let start = starts[idx];
        let end = starts.get(idx + 1).copied().unwrap_or(len + 1) - 1;
        (start, (end + 1).min(len).max(start))
    }
    fn word_bounds(&self, pos: i32) -> (i32, i32) {
        let chars: Vec<char> = self.text.borrow().chars().collect();
        let is_word = |c: char| c.is_alphanumeric() || c == '_';
        let mut s = (pos.max(0) as usize).min(chars.len());
        let mut e = s;
        while s > 0 && is_word(chars[s - 1]) {
            s -= 1;
        }
        while e < chars.len() && is_word(chars[e]) {
            e += 1;
        }
        if e == s && e < chars.len() {
            e += 1; // punctuation / whitespace is its own "word"
        }
        (s as i32, e as i32)
    }
}

// ---------------------------------------------------------------------------------------------
// ITextRangeProvider
// ---------------------------------------------------------------------------------------------
implement_decl! {
    impl TextRange as pub TextRange_Impl: [ITextRangeProvider]
}

pub struct TextRange {
    model: Rc<TextModel>,
    start: Cell<i32>,
    end: Cell<i32>,
    peer: Rc<PeerSlot>,
}

/// The peer of the control (so a range can report its enclosing element).
#[derive(Default)]
pub struct PeerSlot(RefCell<Option<AutomationPeer>>);

fn new_range(model: &Rc<TextModel>, peer: &Rc<PeerSlot>, start: i32, end: i32) -> ITextRangeProvider {
    TextRange { model: model.clone(), start: Cell::new(start), end: Cell::new(end), peer: peer.clone() }.into()
}

fn range_of(r: Ref<ITextRangeProvider>) -> Result<(i32, i32)> {
    let r = r.as_ref().ok_or_else(|| Error::from(E_POINTER))?;
    // SAFETY: every ITextRangeProvider handed to UIA by this module is a `TextRange`.
    let imp: &TextRange = unsafe { windows_core::AsImpl::as_impl(r) };
    Ok((imp.start.get(), imp.end.get()))
}

impl ITextRangeProvider_Impl for TextRange_Impl {
    fn Clone(&self) -> Result<ITextRangeProvider> {
        Ok(new_range(&self.model, &self.peer, self.start.get(), self.end.get()))
    }
    fn Compare(&self, other: Ref<ITextRangeProvider>) -> Result<bool> {
        let (s, e) = range_of(other)?;
        Ok(s == self.start.get() && e == self.end.get())
    }
    fn CompareEndpoints(
        &self,
        endpoint: TextPatternRangeEndpoint,
        other: Ref<ITextRangeProvider>,
        target: TextPatternRangeEndpoint,
    ) -> Result<i32> {
        let (os, oe) = range_of(other)?;
        let mine = if endpoint == TextPatternRangeEndpoint::Start { self.start.get() } else { self.end.get() };
        let theirs = if target == TextPatternRangeEndpoint::Start { os } else { oe };
        Ok(mine - theirs)
    }
    fn ExpandToEnclosingUnit(&self, unit: TextUnit) -> Result<()> {
        let (s, e) = (self.start.get(), self.end.get());
        let (ns, ne) = match unit {
            TextUnit::Character => (s, (s + 1).min(self.model.len())),
            TextUnit::Word => self.model.word_bounds(s),
            TextUnit::Line | TextUnit::Paragraph => self.model.line_bounds(s),
            _ => (0, self.model.len()),
        };
        let _ = e;
        self.start.set(ns);
        self.end.set(ne);
        Ok(())
    }
    fn FindAttribute(&self, _id: i32, _value: Ref<IInspectable>, _backward: bool) -> Result<ITextRangeProvider> {
        Err(Error::from(E_NOTIMPL))
    }
    fn FindText(&self, text: &HSTRING, backward: bool, ignore_case: bool) -> Result<ITextRangeProvider> {
        let hay_full = self.model.slice(self.start.get(), self.end.get());
        let (hay, needle) = if ignore_case {
            (hay_full.to_lowercase(), text.to_string_lossy().to_lowercase())
        } else {
            (hay_full, text.to_string_lossy())
        };
        let found = if backward { hay.rfind(&needle) } else { hay.find(&needle) };
        match found {
            Some(byte) => {
                let chars_before = hay[..byte].chars().count() as i32;
                let s = self.start.get() + chars_before;
                Ok(new_range(&self.model, &self.peer, s, s + needle.chars().count() as i32))
            }
            None => Err(Error::from(E_FAIL)),
        }
    }
    fn GetAttributeValue(&self, _id: i32) -> Result<IInspectable> {
        // "Not supported" sentinel: UIA treats a null value as unsupported.
        Err(Error::from(E_NOTIMPL))
    }
    fn GetBoundingRectangles(&self, out: &mut Array<f64>) -> Result<()> {
        // A real terminal returns one rect per visible line fragment (x, y, w, h in screen pixels).
        *out = Array::new();
        Ok(())
    }
    fn GetEnclosingElement(&self) -> Result<IRawElementProviderSimple> {
        // AutomationPeer.ProviderFromPeer is a protected method (IAutomationPeerProtected on the inner object).
        Err(Error::from(E_NOTIMPL))
    }
    fn GetText(&self, max_length: i32) -> Result<HSTRING> {
        let mut s = self.model.slice(self.start.get(), self.end.get());
        if max_length >= 0 {
            s = s.chars().take(max_length as usize).collect();
        }
        Ok(HSTRING::from(s))
    }
    fn Move(&self, unit: TextUnit, count: i32) -> Result<i32> {
        let len = self.model.len();
        let (s, e) = (self.start.get(), self.end.get());
        let (ns, ne, moved) = match unit {
            TextUnit::Character => {
                let ns = (s + count).clamp(0, len);
                (ns, (ns + (e - s).max(0)).min(len), ns - s)
            }
            TextUnit::Line | TextUnit::Paragraph => {
                let starts = self.model.line_starts();
                let cur = starts.iter().rposition(|x| *x <= s).unwrap_or(0) as i32;
                let target = (cur + count).clamp(0, starts.len() as i32 - 1);
                let (bs, be) = self.model.line_bounds(starts[target as usize]);
                (bs, be, target - cur)
            }
            _ => (0, len, 0),
        };
        self.start.set(ns);
        self.end.set(ne);
        Ok(moved)
    }
    fn MoveEndpointByUnit(&self, endpoint: TextPatternRangeEndpoint, unit: TextUnit, count: i32) -> Result<i32> {
        let len = self.model.len();
        let cell = if endpoint == TextPatternRangeEndpoint::Start { &self.start } else { &self.end };
        let before = cell.get();
        let after = match unit {
            TextUnit::Character => (before + count).clamp(0, len),
            TextUnit::Line | TextUnit::Paragraph => {
                let starts = self.model.line_starts();
                let cur = starts.iter().rposition(|x| *x <= before).unwrap_or(0) as i32;
                let target = (cur + count).clamp(0, starts.len() as i32 - 1);
                starts[target as usize]
            }
            _ => {
                if count < 0 {
                    0
                } else {
                    len
                }
            }
        };
        cell.set(after);
        if self.start.get() > self.end.get() {
            if endpoint == TextPatternRangeEndpoint::Start {
                self.end.set(self.start.get());
            } else {
                self.start.set(self.end.get());
            }
        }
        Ok(match unit {
            TextUnit::Character => after - before,
            _ => count.signum() * (after != before) as i32,
        })
    }
    fn MoveEndpointByRange(
        &self,
        endpoint: TextPatternRangeEndpoint,
        other: Ref<ITextRangeProvider>,
        target: TextPatternRangeEndpoint,
    ) -> Result<()> {
        let (os, oe) = range_of(other)?;
        let v = if target == TextPatternRangeEndpoint::Start { os } else { oe };
        if endpoint == TextPatternRangeEndpoint::Start {
            self.start.set(v);
            if self.end.get() < v {
                self.end.set(v);
            }
        } else {
            self.end.set(v);
            if self.start.get() > v {
                self.start.set(v);
            }
        }
        Ok(())
    }
    fn Select(&self) -> Result<()> {
        self.model.caret.set((self.start.get(), self.end.get()));
        log(&format!("UIA TextRange.Select({}, {})", self.start.get(), self.end.get()));
        Ok(())
    }
    fn AddToSelection(&self) -> Result<()> {
        Err(Error::from(E_NOTIMPL))
    }
    fn RemoveFromSelection(&self) -> Result<()> {
        Err(Error::from(E_NOTIMPL))
    }
    fn ScrollIntoView(&self, _align_to_top: bool) -> Result<()> {
        Ok(())
    }
    fn GetChildren(&self) -> Result<Array<IRawElementProviderSimple>> {
        Ok(Array::new())
    }
}

// ---------------------------------------------------------------------------------------------
// The automation peer: FrameworkElementAutomationPeer + ITextProvider
// ---------------------------------------------------------------------------------------------
implement_decl! {
    impl TerminalPeer as pub TerminalPeer_Impl: [IAutomationPeerOverrides, ITextProvider]
}

pub struct TerminalPeer {
    model: Rc<TextModel>,
    slot: Rc<PeerSlot>,
    name: String,
}

impl TerminalPeer_Impl {
    fn base(&self) -> Result<IAutomationPeerOverrides> {
        self.base.as_option().as_ref().ok_or_else(|| Error::from(E_FAIL))?.cast()
    }
}

macro_rules! forward_peer {
    ($( fn $name:ident(&self $(, $arg:ident : $ty:ty)*) -> $ret:ty; )*) => {
        $( fn $name(&self $(, $arg: $ty)*) -> $ret { self.base()?.$name($($arg),*) } )*
    };
}

/// Same, for getters whose minimal-mode client returns `String` instead of `HSTRING`.
macro_rules! forward_peer_str {
    ($( fn $name:ident; )*) => {
        $( fn $name(&self) -> Result<HSTRING> { self.base()?.$name().map(HSTRING::from) } )*
    };
}

impl IAutomationPeerOverrides_Impl for TerminalPeer_Impl {
    // --- overridden ---------------------------------------------------------------------------
    fn GetPatternCore(&self, pattern: PatternInterface) -> Result<IInspectable> {
        log(&format!("peer.GetPatternCore({})", pattern.0));
        if pattern == PatternInterface::Text {
            return Ok(self.to_interface::<IInspectable>());
        }
        self.base()?.GetPatternCore(pattern)
    }
    fn GetAutomationControlTypeCore(&self) -> Result<AutomationControlType> {
        log("peer.GetAutomationControlTypeCore");
        Ok(AutomationControlType::Document)
    }
    fn GetClassNameCore(&self) -> Result<HSTRING> {
        Ok(HSTRING::from("NekominalTerminal"))
    }
    fn GetNameCore(&self) -> Result<HSTRING> {
        log("peer.GetNameCore");
        Ok(HSTRING::from(self.name.as_str()))
    }
    fn GetLocalizedControlTypeCore(&self) -> Result<HSTRING> {
        Ok(HSTRING::from("終端機"))
    }
    fn IsContentElementCore(&self) -> Result<bool> {
        Ok(true)
    }
    fn IsControlElementCore(&self) -> Result<bool> {
        Ok(true)
    }
    fn IsKeyboardFocusableCore(&self) -> Result<bool> {
        Ok(true)
    }
    fn GetLiveSettingCore(&self) -> Result<AutomationLiveSetting> {
        Ok(AutomationLiveSetting::Polite)
    }

    // --- forwarded to the base peer -----------------------------------------------------------
    forward_peer_str! {
        fn GetAcceleratorKeyCore;
        fn GetAccessKeyCore;
        fn GetAutomationIdCore;
        fn GetHelpTextCore;
        fn GetItemStatusCore;
        fn GetItemTypeCore;
        fn GetLocalizedLandmarkTypeCore;
        fn GetFullDescriptionCore;
    }
    fn GetPeerFromPointCore(&self, point: &Point) -> Result<AutomationPeer> {
        self.base()?.GetPeerFromPointCore(*point)
    }
    fn GetElementFromPointCore(&self, point_in_window: &Point) -> Result<IInspectable> {
        self.base()?.GetElementFromPointCore(*point_in_window)
    }
    forward_peer! {
        fn GetBoundingRectangleCore(&self) -> Result<Rect>;
        fn GetChildrenCore(&self) -> Result<windows_collections::IVector<AutomationPeer>>;
        fn NavigateCore(&self, direction: AutomationNavigationDirection) -> Result<IInspectable>;
        fn GetClickablePointCore(&self) -> Result<Point>;
        fn GetLabeledByCore(&self) -> Result<AutomationPeer>;
        fn GetOrientationCore(&self) -> Result<AutomationOrientation>;
        fn HasKeyboardFocusCore(&self) -> Result<bool>;
        fn IsEnabledCore(&self) -> Result<bool>;
        fn IsOffscreenCore(&self) -> Result<bool>;
        fn IsPasswordCore(&self) -> Result<bool>;
        fn IsRequiredForFormCore(&self) -> Result<bool>;
        fn SetFocusCore(&self) -> Result<()>;
        fn GetFocusedElementCore(&self) -> Result<IInspectable>;
        fn ShowContextMenuCore(&self) -> Result<()>;
        fn GetControlledPeersCore(&self) -> Result<windows_collections::IVectorView<AutomationPeer>>;
        fn GetAnnotationsCore(&self) -> Result<windows_collections::IVector<AutomationPeerAnnotation>>;
        fn GetPositionInSetCore(&self) -> Result<i32>;
        fn GetSizeOfSetCore(&self) -> Result<i32>;
        fn GetLevelCore(&self) -> Result<i32>;
        fn GetLandmarkTypeCore(&self) -> Result<AutomationLandmarkType>;
        fn IsPeripheralCore(&self) -> Result<bool>;
        fn IsDataValidForFormCore(&self) -> Result<bool>;
        fn GetDescribedByCore(&self) -> Result<windows_collections::IIterable<AutomationPeer>>;
        fn GetFlowsToCore(&self) -> Result<windows_collections::IIterable<AutomationPeer>>;
        fn GetFlowsFromCore(&self) -> Result<windows_collections::IIterable<AutomationPeer>>;
        fn GetCultureCore(&self) -> Result<i32>;
        fn GetHeadingLevelCore(&self) -> Result<AutomationHeadingLevel>;
        fn IsDialogCore(&self) -> Result<bool>;
    }
}

impl ITextProvider_Impl for TerminalPeer_Impl {
    fn DocumentRange(&self) -> Result<ITextRangeProvider> {
        Ok(new_range(&self.model, &self.slot, 0, self.model.len()))
    }
    fn SupportedTextSelection(&self) -> Result<SupportedTextSelection> {
        Ok(SupportedTextSelection::Single)
    }
    fn GetSelection(&self) -> Result<Array<ITextRangeProvider>> {
        let (s, e) = self.model.caret.get();
        Ok(Array::from_slice(&[Some(new_range(&self.model, &self.slot, s, e))]))
    }
    fn GetVisibleRanges(&self) -> Result<Array<ITextRangeProvider>> {
        Ok(Array::from_slice(&[Some(new_range(&self.model, &self.slot, 0, self.model.len()))]))
    }
    fn RangeFromChild(&self, _child: Ref<IRawElementProviderSimple>) -> Result<ITextRangeProvider> {
        Err(Error::from(E_NOTIMPL))
    }
    fn RangeFromPoint(&self, _screen_location: &Point) -> Result<ITextRangeProvider> {
        Ok(new_range(&self.model, &self.slot, 0, 0))
    }
}

// ---------------------------------------------------------------------------------------------
// The control: UserControl subclass overriding OnCreateAutomationPeer
// ---------------------------------------------------------------------------------------------
implement_decl! {
    impl TerminalControl as pub TerminalControl_Impl: [IUIElementOverrides]
}

pub struct TerminalControl {
    pub model: Rc<TextModel>,
    pub name: String,
    slot: Rc<PeerSlot>,
    /// Raw (non-owning) pointer of the composed outer UserControl; valid while the control is alive.
    owner: Rc<Cell<*mut core::ffi::c_void>>,
}

impl TerminalControl_Impl {
    fn base(&self) -> Result<IUIElementOverrides> {
        self.base.as_option().as_ref().ok_or_else(|| Error::from(E_FAIL))?.cast()
    }
}

impl IUIElementOverrides_Impl for TerminalControl_Impl {
    fn OnCreateAutomationPeer(&self) -> Result<AutomationPeer> {
        log("OnCreateAutomationPeer called");
        if std::env::var("SPIKE_PEER").is_ok_and(|v| v == "base") {
            // Isolation experiment: delegate to the base UserControl implementation (no custom peer).
            let r = self.base()?.OnCreateAutomationPeer();
            log(&format!("OnCreateAutomationPeer(base) -> {:?}", r.as_ref().map(|_| ())));
            return r;
        }
        let r = self.create_peer();
        if let Err(e) = &r {
            log(&format!("OnCreateAutomationPeer failed: {e}"));
        }
        r
    }
    fn OnDisconnectVisualChildren(&self) -> Result<()> {
        self.base()?.OnDisconnectVisualChildren()
    }
    fn FindSubElementsForTouchTargeting(
        &self,
        point: &Point,
        bounding_rect: &Rect,
    ) -> Result<windows_collections::IIterable<windows_collections::IIterable<Point>>> {
        self.base()?.FindSubElementsForTouchTargeting(*point, *bounding_rect)
    }
    fn GetChildrenInTabFocusOrder(&self) -> Result<windows_collections::IIterable<DependencyObject>> {
        self.base()?.GetChildrenInTabFocusOrder()
    }
    fn OnKeyboardAcceleratorInvoked(&self, args: Ref<KeyboardAcceleratorInvokedEventArgs>) -> Result<()> {
        self.base()?.OnKeyboardAcceleratorInvoked(args.as_ref())
    }
    fn OnProcessKeyboardAccelerators(&self, args: Ref<ProcessKeyboardAcceleratorEventArgs>) -> Result<()> {
        self.base()?.OnProcessKeyboardAccelerators(args.as_ref())
    }
    fn OnBringIntoViewRequested(&self, e: Ref<BringIntoViewRequestedEventArgs>) -> Result<()> {
        self.base()?.OnBringIntoViewRequested(e.as_ref())
    }
    fn PopulatePropertyInfoOverride(&self, name: &HSTRING, info: Ref<AnimationPropertyInfo>) -> Result<()> {
        self.base()?.PopulatePropertyInfoOverride(&name.to_string_lossy(), info.as_ref())
    }
}

impl TerminalControl_Impl {
    fn create_peer(&self) -> Result<AutomationPeer> {
        if let Some(existing) = self.slot.0.borrow().clone() {
            return Ok(existing);
        }
        let raw = self.owner.get();
        log(&format!("create_peer: owner raw = {raw:?}"));
        // SAFETY: the pointer was stored from the live outer object right after composition.
        let owner: FrameworkElement =
            unsafe { windows_core::IUnknown::from_raw_borrowed(&raw) }.ok_or_else(|| Error::from(E_FAIL))?.cast()?;
        log("create_peer: owner cloned, composing peer");
        let peer = FrameworkElementAutomationPeer::CreateInstanceWithOwner_compose(
            &owner,
            TerminalPeer { model: self.model.clone(), slot: self.slot.clone(), name: self.name.clone() },
        )?;
        log("create_peer: peer composed");
        let peer: AutomationPeer = peer.cast()?;
        *self.slot.0.borrow_mut() = Some(peer.clone());
        log("OnCreateAutomationPeer -> TerminalPeer");
        Ok(peer)
    }
}
/// Creates the composed control.  `model` is shared with the renderer side.
pub fn create(name: &str, model: Rc<TextModel>) -> Result<UserControl> {
    let owner = Rc::new(Cell::new(core::ptr::null_mut()));
    let control =
        TerminalControl { model, name: name.to_string(), slot: Rc::new(PeerSlot::default()), owner: owner.clone() };
    let uc = UserControl::compose(control)?;
    // Identity (IUnknown) pointer, not the IUserControl interface pointer: it is re-queried for the right interface later.
    let identity: windows_core::IUnknown = uc.cast()?;
    owner.set(identity.as_raw());
    Ok(uc)
}
