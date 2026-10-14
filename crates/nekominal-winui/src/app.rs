//! Process bootstrap and the `Application` subclass (pattern from windows-reactor `native/{bootstrap,app_shim}.rs`,
//! MIT/Apache-2.0, proven in winui3-rust.md route B).
//!
//! `run` does: dynamic dependency on the Windows App Runtime (framework-dependent, unpackaged), per-monitor-v2
//! DPI awareness, STA, `Application::Start`, and merges `XamlControlsResources` before `on_launched`.

use std::cell::RefCell;

use nekominal_winui_sys::*;
use windows_core::*;

use crate::diag::log;

const FRAMEWORK_FAMILY: PCWSTR = w!("Microsoft.WindowsAppRuntime.2_8wekyb3d8bbwe");

fn bootstrap() -> Result<()> {
    let mut dependency_id = PWSTR::null();
    unsafe {
        TryCreatePackageDependency(
            std::ptr::null_mut(),
            FRAMEWORK_FAMILY,
            PACKAGE_VERSION { Anonymous: PACKAGE_VERSION_0 { Version: WINDOWSAPPSDK_RUNTIME_VERSION_UINT64 } },
            PackageDependencyProcessorArchitectures_X64 | PackageDependencyProcessorArchitectures_Neutral,
            0, // PackageDependencyLifetimeKind_Process
            PCWSTR::null(),
            0,
            &mut dependency_id,
        )
        .ok()?;
        let mut context = std::ptr::null_mut();
        let mut full_name = PWSTR::null();
        AddPackageDependency(PCWSTR(dependency_id.0), 0, 0, &mut context, &mut full_name).ok()?;
        log(&format!("bootstrapped: {}", full_name.to_string().unwrap_or_default()));
        let _ = HeapFree(GetProcessHeap(), 0, dependency_id.0.cast());
        let _ = HeapFree(GetProcessHeap(), 0, full_name.0.cast());
    }
    Ok(())
}

implement_decl! {
    impl XamlApp as pub XamlApp_Impl: [IApplicationOverrides, IXamlMetadataProvider]
}

type OnLaunched = Box<dyn FnOnce() -> Result<()>>;

pub struct XamlApp {
    provider: RefCell<Option<XamlControlsXamlMetaDataProvider>>,
    on_launched: RefCell<Option<OnLaunched>>,
}

impl XamlApp {
    fn provider(&self) -> Result<XamlControlsXamlMetaDataProvider> {
        if let Some(p) = self.provider.borrow().as_ref() {
            return Ok(p.clone());
        }
        let p = XamlControlsXamlMetaDataProvider::new()?;
        *self.provider.borrow_mut() = Some(p.clone());
        Ok(p)
    }
}

impl IApplicationOverrides_Impl for XamlApp_Impl {
    fn OnLaunched(&self, _args: Ref<LaunchActivatedEventArgs>) -> Result<()> {
        // Without the merged WinUI control resources the controls have no Fluent styles.
        Application::Current()?.Resources()?.MergedDictionaries()?.Append(&XamlControlsResources::new()?.cast::<ResourceDictionary>()?)?;
        if let Some(on_launched) = self.on_launched.borrow_mut().take() {
            on_launched()?;
        }
        Ok(())
    }
}

impl IXamlMetadataProvider_Impl for XamlApp_Impl {
    fn GetXamlType(&self, ty: &TypeName) -> Result<IXamlType> {
        self.provider()?.GetXamlType(ty)
    }
    fn GetXamlTypeByFullName(&self, full_name: &HSTRING) -> Result<IXamlType> {
        self.provider()?.GetXamlTypeByFullName(&full_name.to_string_lossy())
    }
    fn GetXmlnsDefinitions(&self) -> Result<Array<XmlnsDefinition>> {
        self.provider()?.GetXmlnsDefinitions()
    }
}

thread_local! {
    static APP: RefCell<Option<Application>> = const { RefCell::new(None) };
}

/// Runs the XAML message loop on this thread until the application exits. `on_launched` builds the first window.
pub fn run(on_launched: impl FnOnce() -> Result<()> + 'static) -> Result<()> {
    crate::diag::init();
    bootstrap()?;
    unsafe {
        let _ = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
        CoInitializeEx(std::ptr::null(), COINIT_APARTMENTTHREADED as u32).ok()?;
    }
    let on_launched = RefCell::new(Some(on_launched));
    Application::Start(&ApplicationInitializationCallback::new(move |_| {
        let on_launched = on_launched.borrow_mut().take();
        match Application::compose(XamlApp {
            provider: RefCell::new(None),
            on_launched: RefCell::new(on_launched.map(|f| Box::new(f) as OnLaunched)),
        }) {
            Ok(app) => APP.with(|a| *a.borrow_mut() = Some(app)),
            Err(e) => log(&format!("FATAL compose Application: {e}")),
        }
    }))
}
