//! The standard COM registrations of a Windows machine below `Classes` (ADR 0186 addendum,
//! `spec-win-wow64-defaults`): the 82 interfaces, the 27 proxy/stub and marshaller classes per
//! view, and the standard type library `stdole`. **Generated** from the recorded outputs of the
//! sheets `ComDefaults.Interfaces`, `ComDefaults.ProxyStubClasses` and `ComDefaults.StdOleTypeLib`
//! (names and values a test program read on the oracle; `docs/windows/registry-defaults.md`
//! section 5); paths are relative to a `Classes` key. Keys listed without values are the
//! subkeys the oracle has (their contents are not part of the sheets).

use super::defaults::Data::{ExpandSz, Sz};
use super::defaults::{DefaultKey, k};

/// The interfaces (identical in both views): `Interface\{iid}` and its subkeys.
pub const INTERFACES: &[DefaultKey] = &[
    k("Interface\\{00000000-0000-0000-C000-000000000046}", &[("", Sz("IUnknown"))]),
    k("Interface\\{00000001-0000-0000-C000-000000000046}", &[("", Sz("IClassFactory"))]),
    k(
        "Interface\\{00000001-0000-0000-C000-000000000046}\\ProxyStubClsid32",
        &[("", Sz("{00000320-0000-0000-C000-000000000046}"))],
    ),
    k("Interface\\{0000000A-0000-0000-C000-000000000046}", &[("", Sz("ILockBytes"))]),
    k(
        "Interface\\{0000000A-0000-0000-C000-000000000046}\\ProxyStubClsid32",
        &[("", Sz("{00000320-0000-0000-C000-000000000046}"))],
    ),
    k("Interface\\{0000000B-0000-0000-C000-000000000046}", &[("", Sz("IStorage"))]),
    k(
        "Interface\\{0000000B-0000-0000-C000-000000000046}\\ProxyStubClsid32",
        &[("", Sz("{00000320-0000-0000-C000-000000000046}"))],
    ),
    k("Interface\\{0000000C-0000-0000-C000-000000000046}", &[("", Sz("IStream"))]),
    k(
        "Interface\\{0000000C-0000-0000-C000-000000000046}\\ProxyStubClsid32",
        &[("", Sz("{00000320-0000-0000-C000-000000000046}"))],
    ),
    k("Interface\\{0000000D-0000-0000-C000-000000000046}", &[("", Sz("IEnumSTATSTG"))]),
    k(
        "Interface\\{0000000D-0000-0000-C000-000000000046}\\ProxyStubClsid32",
        &[("", Sz("{00000320-0000-0000-C000-000000000046}"))],
    ),
    k("Interface\\{0000000E-0000-0000-C000-000000000046}", &[("", Sz("IBindCtx"))]),
    k(
        "Interface\\{0000000E-0000-0000-C000-000000000046}\\ProxyStubClsid32",
        &[("", Sz("{00000320-0000-0000-C000-000000000046}"))],
    ),
    k("Interface\\{0000000F-0000-0000-C000-000000000046}", &[("", Sz("IMoniker"))]),
    k(
        "Interface\\{0000000F-0000-0000-C000-000000000046}\\ProxyStubClsid32",
        &[("", Sz("{00000320-0000-0000-C000-000000000046}"))],
    ),
    k("Interface\\{00000010-0000-0000-C000-000000000046}", &[("", Sz("IRunningObjectTable"))]),
    k(
        "Interface\\{00000010-0000-0000-C000-000000000046}\\ProxyStubClsid32",
        &[("", Sz("{00000320-0000-0000-C000-000000000046}"))],
    ),
    k("Interface\\{00000012-0000-0000-C000-000000000046}", &[("", Sz("IRootStorage"))]),
    k(
        "Interface\\{00000012-0000-0000-C000-000000000046}\\ProxyStubClsid32",
        &[("", Sz("{00000320-0000-0000-C000-000000000046}"))],
    ),
    k("Interface\\{00000022-0000-0000-C000-000000000046}", &[("", Sz("ISurrogate"))]),
    k(
        "Interface\\{00000022-0000-0000-C000-000000000046}\\ProxyStubClsid32",
        &[("", Sz("{00000320-0000-0000-C000-000000000046}"))],
    ),
    k("Interface\\{00000026-0000-0000-C000-000000000046}", &[("", Sz("IUrlMon"))]),
    k(
        "Interface\\{00000026-0000-0000-C000-000000000046}\\ProxyStubClsid32",
        &[("", Sz("{00000320-0000-0000-C000-000000000046}"))],
    ),
    k("Interface\\{00000030-0000-0000-C000-000000000046}", &[("", Sz("ISynchronize"))]),
    k(
        "Interface\\{00000030-0000-0000-C000-000000000046}\\ProxyStubClsid32",
        &[("", Sz("{00000320-0000-0000-C000-000000000046}"))],
    ),
    k("Interface\\{00000035-0000-0000-C000-000000000046}", &[("", Sz("IActivationFactory"))]),
    k(
        "Interface\\{00000035-0000-0000-C000-000000000046}\\ProxyStubClsid32",
        &[("", Sz("{00000320-0000-0000-C000-000000000046}"))],
    ),
    k("Interface\\{00000036-0000-0000-C000-000000000046}", &[("", Sz("IAsyncInfo"))]),
    k(
        "Interface\\{00000036-0000-0000-C000-000000000046}\\ProxyStubClsid32",
        &[("", Sz("{00000320-0000-0000-C000-000000000046}"))],
    ),
    k("Interface\\{00000037-0000-0000-C000-000000000046}", &[("", Sz("IWeakReference"))]),
    k(
        "Interface\\{00000037-0000-0000-C000-000000000046}\\ProxyStubClsid32",
        &[("", Sz("{00000320-0000-0000-C000-000000000046}"))],
    ),
    k("Interface\\{00000038-0000-0000-C000-000000000046}", &[("", Sz("IWeakReferenceSource"))]),
    k(
        "Interface\\{00000038-0000-0000-C000-000000000046}\\ProxyStubClsid32",
        &[("", Sz("{00000320-0000-0000-C000-000000000046}"))],
    ),
    k("Interface\\{00000100-0000-0000-C000-000000000046}", &[("", Sz("IEnumUnknown"))]),
    k(
        "Interface\\{00000100-0000-0000-C000-000000000046}\\ProxyStubClsid32",
        &[("", Sz("{00000320-0000-0000-C000-000000000046}"))],
    ),
    k("Interface\\{00000101-0000-0000-C000-000000000046}", &[("", Sz("IEnumString"))]),
    k(
        "Interface\\{00000101-0000-0000-C000-000000000046}\\ProxyStubClsid32",
        &[("", Sz("{00000320-0000-0000-C000-000000000046}"))],
    ),
    k("Interface\\{00000102-0000-0000-C000-000000000046}", &[("", Sz("IEnumMoniker"))]),
    k(
        "Interface\\{00000102-0000-0000-C000-000000000046}\\ProxyStubClsid32",
        &[("", Sz("{00000320-0000-0000-C000-000000000046}"))],
    ),
    k("Interface\\{00000103-0000-0000-C000-000000000046}", &[("", Sz("IEnumFORMATETC"))]),
    k(
        "Interface\\{00000103-0000-0000-C000-000000000046}\\ProxyStubClsid32",
        &[("", Sz("{00000320-0000-0000-C000-000000000046}"))],
    ),
    k("Interface\\{00000104-0000-0000-C000-000000000046}", &[("", Sz("IEnumOLEVERB"))]),
    k(
        "Interface\\{00000104-0000-0000-C000-000000000046}\\ProxyStubClsid32",
        &[("", Sz("{00000320-0000-0000-C000-000000000046}"))],
    ),
    k("Interface\\{00000105-0000-0000-C000-000000000046}", &[("", Sz("IEnumSTATDATA"))]),
    k(
        "Interface\\{00000105-0000-0000-C000-000000000046}\\ProxyStubClsid32",
        &[("", Sz("{00000320-0000-0000-C000-000000000046}"))],
    ),
    k("Interface\\{00000109-0000-0000-C000-000000000046}", &[("", Sz("IPersistStream"))]),
    k(
        "Interface\\{00000109-0000-0000-C000-000000000046}\\ProxyStubClsid32",
        &[("", Sz("{00000320-0000-0000-C000-000000000046}"))],
    ),
    k("Interface\\{0000010A-0000-0000-C000-000000000046}", &[("", Sz("IPersistStorage"))]),
    k(
        "Interface\\{0000010A-0000-0000-C000-000000000046}\\ProxyStubClsid32",
        &[("", Sz("{00000320-0000-0000-C000-000000000046}"))],
    ),
    k("Interface\\{0000010B-0000-0000-C000-000000000046}", &[("", Sz("IPersistFile"))]),
    k(
        "Interface\\{0000010B-0000-0000-C000-000000000046}\\ProxyStubClsid32",
        &[("", Sz("{00000320-0000-0000-C000-000000000046}"))],
    ),
    k("Interface\\{0000010C-0000-0000-C000-000000000046}", &[("", Sz("IPersist"))]),
    k(
        "Interface\\{0000010C-0000-0000-C000-000000000046}\\ProxyStubClsid32",
        &[("", Sz("{00000320-0000-0000-C000-000000000046}"))],
    ),
    k("Interface\\{0000010D-0000-0000-C000-000000000046}", &[("", Sz("IViewObject"))]),
    k(
        "Interface\\{0000010D-0000-0000-C000-000000000046}\\ProxyStubClsid32",
        &[("", Sz("{00000320-0000-0000-C000-000000000046}"))],
    ),
    k("Interface\\{0000010E-0000-0000-C000-000000000046}", &[("", Sz("IDataObject"))]),
    k(
        "Interface\\{0000010E-0000-0000-C000-000000000046}\\ProxyStubClsid32",
        &[("", Sz("{00000320-0000-0000-C000-000000000046}"))],
    ),
    k("Interface\\{0000010F-0000-0000-C000-000000000046}", &[("", Sz("IAdviseSink"))]),
    k(
        "Interface\\{0000010F-0000-0000-C000-000000000046}\\ProxyStubClsid32",
        &[("", Sz("{00000320-0000-0000-C000-000000000046}"))],
    ),
    k(
        "Interface\\{0000010F-0000-0000-C000-000000000046}\\AsynchronousInterface",
        &[("", Sz("{00000150-0000-0000-C000-000000000046}"))],
    ),
    k("Interface\\{00000112-0000-0000-C000-000000000046}", &[("", Sz("IOleObject"))]),
    k(
        "Interface\\{00000112-0000-0000-C000-000000000046}\\ProxyStubClsid32",
        &[("", Sz("{00000320-0000-0000-C000-000000000046}"))],
    ),
    k("Interface\\{00000113-0000-0000-C000-000000000046}", &[("", Sz("IOleInPlaceObject"))]),
    k(
        "Interface\\{00000113-0000-0000-C000-000000000046}\\ProxyStubClsid32",
        &[("", Sz("{00000320-0000-0000-C000-000000000046}"))],
    ),
    k("Interface\\{00000114-0000-0000-C000-000000000046}", &[("", Sz("IOleWindow"))]),
    k(
        "Interface\\{00000114-0000-0000-C000-000000000046}\\ProxyStubClsid32",
        &[("", Sz("{00000320-0000-0000-C000-000000000046}"))],
    ),
    k("Interface\\{00000115-0000-0000-C000-000000000046}", &[("", Sz("IOleInPlaceUIWindow"))]),
    k(
        "Interface\\{00000115-0000-0000-C000-000000000046}\\ProxyStubClsid32",
        &[("", Sz("{00000320-0000-0000-C000-000000000046}"))],
    ),
    k("Interface\\{00000116-0000-0000-C000-000000000046}", &[("", Sz("IOleInPlaceFrame"))]),
    k(
        "Interface\\{00000116-0000-0000-C000-000000000046}\\ProxyStubClsid32",
        &[("", Sz("{00000320-0000-0000-C000-000000000046}"))],
    ),
    k("Interface\\{00000117-0000-0000-C000-000000000046}", &[("", Sz("IOleInPlaceActiveObject"))]),
    k(
        "Interface\\{00000117-0000-0000-C000-000000000046}\\ProxyStubClsid32",
        &[("", Sz("{00000320-0000-0000-C000-000000000046}"))],
    ),
    k("Interface\\{00000118-0000-0000-C000-000000000046}", &[("", Sz("IOleClientSite"))]),
    k(
        "Interface\\{00000118-0000-0000-C000-000000000046}\\ProxyStubClsid32",
        &[("", Sz("{00000320-0000-0000-C000-000000000046}"))],
    ),
    k("Interface\\{00000119-0000-0000-C000-000000000046}", &[("", Sz("IOleInPlaceSite"))]),
    k(
        "Interface\\{00000119-0000-0000-C000-000000000046}\\ProxyStubClsid32",
        &[("", Sz("{00000320-0000-0000-C000-000000000046}"))],
    ),
    k("Interface\\{0000011A-0000-0000-C000-000000000046}", &[("", Sz("IParseDisplayName"))]),
    k(
        "Interface\\{0000011A-0000-0000-C000-000000000046}\\ProxyStubClsid32",
        &[("", Sz("{00000320-0000-0000-C000-000000000046}"))],
    ),
    k("Interface\\{0000011B-0000-0000-C000-000000000046}", &[("", Sz("IOleContainer"))]),
    k(
        "Interface\\{0000011B-0000-0000-C000-000000000046}\\ProxyStubClsid32",
        &[("", Sz("{00000320-0000-0000-C000-000000000046}"))],
    ),
    k("Interface\\{0000011C-0000-0000-C000-000000000046}", &[("", Sz("IOleItemContainer"))]),
    k(
        "Interface\\{0000011C-0000-0000-C000-000000000046}\\ProxyStubClsid32",
        &[("", Sz("{00000320-0000-0000-C000-000000000046}"))],
    ),
    k("Interface\\{0000011D-0000-0000-C000-000000000046}", &[("", Sz("IOleLink"))]),
    k(
        "Interface\\{0000011D-0000-0000-C000-000000000046}\\ProxyStubClsid32",
        &[("", Sz("{00000320-0000-0000-C000-000000000046}"))],
    ),
    k("Interface\\{0000011E-0000-0000-C000-000000000046}", &[("", Sz("IOleCache"))]),
    k(
        "Interface\\{0000011E-0000-0000-C000-000000000046}\\ProxyStubClsid32",
        &[("", Sz("{00000320-0000-0000-C000-000000000046}"))],
    ),
    k("Interface\\{00000122-0000-0000-C000-000000000046}", &[("", Sz("IDropTarget"))]),
    k(
        "Interface\\{00000122-0000-0000-C000-000000000046}\\ProxyStubClsid32",
        &[("", Sz("{00000320-0000-0000-C000-000000000046}"))],
    ),
    k("Interface\\{00000125-0000-0000-C000-000000000046}", &[("", Sz("IAdviseSink2"))]),
    k(
        "Interface\\{00000125-0000-0000-C000-000000000046}\\ProxyStubClsid32",
        &[("", Sz("{00000320-0000-0000-C000-000000000046}"))],
    ),
    k(
        "Interface\\{00000125-0000-0000-C000-000000000046}\\AsynchronousInterface",
        &[("", Sz("{00000151-0000-0000-C000-000000000046}"))],
    ),
    k("Interface\\{00000126-0000-0000-C000-000000000046}", &[("", Sz("IRunnableObject"))]),
    k(
        "Interface\\{00000126-0000-0000-C000-000000000046}\\ProxyStubClsid32",
        &[("", Sz("{00000320-0000-0000-C000-000000000046}"))],
    ),
    k("Interface\\{00000127-0000-0000-C000-000000000046}", &[("", Sz("IViewObject2"))]),
    k(
        "Interface\\{00000127-0000-0000-C000-000000000046}\\ProxyStubClsid32",
        &[("", Sz("{00000320-0000-0000-C000-000000000046}"))],
    ),
    k("Interface\\{00000128-0000-0000-C000-000000000046}", &[("", Sz("IOleCache2"))]),
    k(
        "Interface\\{00000128-0000-0000-C000-000000000046}\\ProxyStubClsid32",
        &[("", Sz("{00000320-0000-0000-C000-000000000046}"))],
    ),
    k("Interface\\{00000129-0000-0000-C000-000000000046}", &[("", Sz("IOleCacheControl"))]),
    k(
        "Interface\\{00000129-0000-0000-C000-000000000046}\\ProxyStubClsid32",
        &[("", Sz("{00000320-0000-0000-C000-000000000046}"))],
    ),
    k("Interface\\{0000012A-0000-0000-C000-000000000046}", &[("", Sz("IContinue"))]),
    k(
        "Interface\\{0000012A-0000-0000-C000-000000000046}\\ProxyStubClsid32",
        &[("", Sz("{00000320-0000-0000-C000-000000000046}"))],
    ),
    k("Interface\\{00000131-0000-0000-C000-000000000046}", &[("", Sz("IRemUnknown"))]),
    k(
        "Interface\\{00000131-0000-0000-C000-000000000046}\\ProxyStubClsid32",
        &[("", Sz("{00000320-0000-0000-C000-000000000046}"))],
    ),
    k(
        "Interface\\{00000131-0000-0000-C000-000000000046}\\AsynchronousInterface",
        &[("", Sz("{000E0131-0000-0000-C000-000000000046}"))],
    ),
    k("Interface\\{00000132-0000-0000-C000-000000000046}", &[("", Sz("ILocalSystemActivator"))]),
    k(
        "Interface\\{00000132-0000-0000-C000-000000000046}\\ProxyStubClsid32",
        &[("", Sz("{00000320-0000-0000-C000-000000000046}"))],
    ),
    k("Interface\\{00000134-0000-0000-C000-000000000046}", &[("", Sz("IRundown"))]),
    k(
        "Interface\\{00000134-0000-0000-C000-000000000046}\\ProxyStubClsid32",
        &[("", Sz("{00000320-0000-0000-C000-000000000046}"))],
    ),
    k(
        "Interface\\{00000134-0000-0000-C000-000000000046}\\AsynchronousInterface",
        &[("", Sz("{000E0134-0000-0000-C000-000000000046}"))],
    ),
    k("Interface\\{00000135-0000-0000-C000-000000000046}", &[("", Sz("IInterfaceFromWindowProp"))]),
    k(
        "Interface\\{00000135-0000-0000-C000-000000000046}\\ProxyStubClsid32",
        &[("", Sz("{00000320-0000-0000-C000-000000000046}"))],
    ),
    k("Interface\\{00000136-0000-0000-C000-000000000046}", &[("", Sz("ISCMLocalActivator"))]),
    k(
        "Interface\\{00000136-0000-0000-C000-000000000046}\\ProxyStubClsid32",
        &[("", Sz("{00000320-0000-0000-C000-000000000046}"))],
    ),
    k("Interface\\{00000138-0000-0000-C000-000000000046}", &[("", Sz("IPropertyStorage"))]),
    k(
        "Interface\\{00000138-0000-0000-C000-000000000046}\\ProxyStubClsid32",
        &[("", Sz("{00000320-0000-0000-C000-000000000046}"))],
    ),
    k("Interface\\{00000139-0000-0000-C000-000000000046}", &[("", Sz("IEnumSTATPROPSTG"))]),
    k(
        "Interface\\{00000139-0000-0000-C000-000000000046}\\ProxyStubClsid32",
        &[("", Sz("{00000320-0000-0000-C000-000000000046}"))],
    ),
    k("Interface\\{0000013A-0000-0000-C000-000000000046}", &[("", Sz("IPropertySetStorage"))]),
    k(
        "Interface\\{0000013A-0000-0000-C000-000000000046}\\ProxyStubClsid32",
        &[("", Sz("{00000320-0000-0000-C000-000000000046}"))],
    ),
    k("Interface\\{0000013B-0000-0000-C000-000000000046}", &[("", Sz("IEnumSTATPROPSETSTG"))]),
    k(
        "Interface\\{0000013B-0000-0000-C000-000000000046}\\ProxyStubClsid32",
        &[("", Sz("{00000320-0000-0000-C000-000000000046}"))],
    ),
    k("Interface\\{0000013C-0000-0000-C000-000000000046}", &[("", Sz("IRemUnknownN"))]),
    k(
        "Interface\\{0000013C-0000-0000-C000-000000000046}\\ProxyStubClsid32",
        &[("", Sz("{00000320-0000-0000-C000-000000000046}"))],
    ),
    k(
        "Interface\\{0000013C-0000-0000-C000-000000000046}\\AsynchronousInterface",
        &[("", Sz("{000B013C-0000-0000-C000-000000000046}"))],
    ),
    k("Interface\\{00000140-0000-0000-C000-000000000046}", &[("", Sz("IClassActivator"))]),
    k(
        "Interface\\{00000140-0000-0000-C000-000000000046}\\ProxyStubClsid32",
        &[("", Sz("{00000320-0000-0000-C000-000000000046}"))],
    ),
    k("Interface\\{00000141-0000-0000-C000-000000000046}", &[("", Sz("IDLLHost"))]),
    k(
        "Interface\\{00000141-0000-0000-C000-000000000046}\\ProxyStubClsid32",
        &[("", Sz("{00000320-0000-0000-C000-000000000046}"))],
    ),
    k("Interface\\{00000142-0000-0000-C000-000000000046}", &[("", Sz("IRemoteQI"))]),
    k(
        "Interface\\{00000142-0000-0000-C000-000000000046}\\ProxyStubClsid32",
        &[("", Sz("{00000320-0000-0000-C000-000000000046}"))],
    ),
    k("Interface\\{00000143-0000-0000-C000-000000000046}", &[("", Sz("IRemUnknown2"))]),
    k(
        "Interface\\{00000143-0000-0000-C000-000000000046}\\ProxyStubClsid32",
        &[("", Sz("{00000320-0000-0000-C000-000000000046}"))],
    ),
    k(
        "Interface\\{00000143-0000-0000-C000-000000000046}\\AsynchronousInterface",
        &[("", Sz("{000E0143-0000-0000-C000-000000000046}"))],
    ),
    k("Interface\\{00000150-0000-0000-C000-000000000046}", &[("", Sz("AsyncIAdviseSink"))]),
    k(
        "Interface\\{00000150-0000-0000-C000-000000000046}\\SynchronousInterface",
        &[("", Sz("{0000010F-0000-0000-C000-000000000046}"))],
    ),
    k("Interface\\{00000151-0000-0000-C000-000000000046}", &[("", Sz("AsyncIAdviseSink2"))]),
    k(
        "Interface\\{00000151-0000-0000-C000-000000000046}\\SynchronousInterface",
        &[("", Sz("{00000125-0000-0000-C000-000000000046}"))],
    ),
    k("Interface\\{00000160-0000-0000-C000-000000000046}", &[("", Sz("IPrivDragDrop"))]),
    k(
        "Interface\\{00000160-0000-0000-C000-000000000046}\\ProxyStubClsid32",
        &[("", Sz("{00000320-0000-0000-C000-000000000046}"))],
    ),
    k("Interface\\{000001A0-0000-0000-C000-000000000046}", &[("", Sz("ISystemActivator"))]),
    k(
        "Interface\\{000001A0-0000-0000-C000-000000000046}\\ProxyStubClsid32",
        &[("", Sz("{00000320-0000-0000-C000-000000000046}"))],
    ),
    k("Interface\\{00020400-0000-0000-C000-000000000046}", &[("", Sz("IDispatch"))]),
    k(
        "Interface\\{00020400-0000-0000-C000-000000000046}\\ProxyStubClsid32",
        &[("", Sz("{00020420-0000-0000-C000-000000000046}"))],
    ),
    k(
        "Interface\\{00020400-0000-0000-C000-000000000046}\\ProxyStubClsid",
        &[("", Sz("{00020420-0000-0000-C000-000000000046}"))],
    ),
    k("Interface\\{00020400-0000-0000-C000-000000000046}\\NumMethods", &[("", Sz("7"))]),
    k("Interface\\{00020401-0000-0000-C000-000000000046}", &[("", Sz("ITypeInfo"))]),
    k(
        "Interface\\{00020401-0000-0000-C000-000000000046}\\ProxyStubClsid32",
        &[("", Sz("{00020422-0000-0000-C000-000000000046}"))],
    ),
    k(
        "Interface\\{00020401-0000-0000-C000-000000000046}\\ProxyStubClsid",
        &[("", Sz("{00020422-0000-0000-C000-000000000046}"))],
    ),
    k("Interface\\{00020401-0000-0000-C000-000000000046}\\NumMethods", &[("", Sz("22"))]),
    k("Interface\\{00020402-0000-0000-C000-000000000046}", &[("", Sz("ITypeLib"))]),
    k(
        "Interface\\{00020402-0000-0000-C000-000000000046}\\ProxyStubClsid32",
        &[("", Sz("{00020423-0000-0000-C000-000000000046}"))],
    ),
    k(
        "Interface\\{00020402-0000-0000-C000-000000000046}\\ProxyStubClsid",
        &[("", Sz("{00020423-0000-0000-C000-000000000046}"))],
    ),
    k("Interface\\{00020402-0000-0000-C000-000000000046}\\NumMethods", &[("", Sz("13"))]),
    k("Interface\\{00020403-0000-0000-C000-000000000046}", &[("", Sz("ITypeComp"))]),
    k(
        "Interface\\{00020403-0000-0000-C000-000000000046}\\ProxyStubClsid32",
        &[("", Sz("{00020425-0000-0000-C000-000000000046}"))],
    ),
    k(
        "Interface\\{00020403-0000-0000-C000-000000000046}\\ProxyStubClsid",
        &[("", Sz("{00020425-0000-0000-C000-000000000046}"))],
    ),
    k("Interface\\{00020403-0000-0000-C000-000000000046}\\NumMethods", &[("", Sz("5"))]),
    k("Interface\\{00020404-0000-0000-C000-000000000046}", &[("", Sz("IEnumVARIANT"))]),
    k(
        "Interface\\{00020404-0000-0000-C000-000000000046}\\ProxyStubClsid32",
        &[("", Sz("{00020421-0000-0000-C000-000000000046}"))],
    ),
    k(
        "Interface\\{00020404-0000-0000-C000-000000000046}\\ProxyStubClsid",
        &[("", Sz("{00020421-0000-0000-C000-000000000046}"))],
    ),
    k("Interface\\{00020404-0000-0000-C000-000000000046}\\NumMethods", &[("", Sz("7"))]),
    k("Interface\\{00020411-0000-0000-C000-000000000046}", &[("", Sz("ITypeLib2"))]),
    k(
        "Interface\\{00020411-0000-0000-C000-000000000046}\\ProxyStubClsid32",
        &[("", Sz("{00020420-0000-0000-C000-000000000046}"))],
    ),
    k("Interface\\{00020411-0000-0000-C000-000000000046}\\NumMethods", &[("", Sz("16"))]),
    k("Interface\\{00020412-0000-0000-C000-000000000046}", &[("", Sz("ITypeInfo2"))]),
    k(
        "Interface\\{00020412-0000-0000-C000-000000000046}\\ProxyStubClsid32",
        &[("", Sz("{00020420-0000-0000-C000-000000000046}"))],
    ),
    k("Interface\\{00020412-0000-0000-C000-000000000046}\\NumMethods", &[("", Sz("32"))]),
    k("Interface\\{1CF2B120-547D-101B-8E65-08002B2BD119}", &[("", Sz("IErrorInfo"))]),
    k(
        "Interface\\{1CF2B120-547D-101B-8E65-08002B2BD119}\\ProxyStubClsid32",
        &[("", Sz("{B196B286-BAB4-101A-B69C-00AA00341D07}"))],
    ),
    k("Interface\\{22F03340-547D-101B-8E65-08002B2BD119}", &[("", Sz("ICreateErrorInfo"))]),
    k(
        "Interface\\{22F03340-547D-101B-8E65-08002B2BD119}\\ProxyStubClsid32",
        &[("", Sz("{B196B286-BAB4-101A-B69C-00AA00341D07}"))],
    ),
    k("Interface\\{22F03340-547D-101B-8E65-08002B2BD119}\\NumMethods", &[("", Sz("8"))]),
    k("Interface\\{DF0B3D60-548F-101B-8E65-08002B2BD119}", &[("", Sz("ISupportErrorInfo"))]),
    k(
        "Interface\\{DF0B3D60-548F-101B-8E65-08002B2BD119}\\ProxyStubClsid32",
        &[("", Sz("{DF0B3D60-548F-101B-8E65-08002B2BD119}"))],
    ),
    k(
        "Interface\\{DF0B3D60-548F-101B-8E65-08002B2BD119}\\ProxyStubClsid",
        &[("", Sz("{DF0B3D60-548F-101B-8E65-08002B2BD119}"))],
    ),
    k("Interface\\{A6EF9860-C720-11D0-9337-00A0C90DCAA9}", &[("", Sz("IDispatchEx"))]),
    k(
        "Interface\\{A6EF9860-C720-11D0-9337-00A0C90DCAA9}\\ProxyStubClsid32",
        &[("", Sz("{10E2414A-EC59-49D2-BC51-5ADD2C36FEBC}"))],
    ),
    k("Interface\\{A6EF9860-C720-11D0-9337-00A0C90DCAA9}\\NumMethods", &[("", Sz("15"))]),
    k("Interface\\{B196B286-BAB4-101A-B69C-00AA00341D07}", &[("", Sz("IConnectionPoint"))]),
    k(
        "Interface\\{B196B286-BAB4-101A-B69C-00AA00341D07}\\ProxyStubClsid32",
        &[("", Sz("{00000320-0000-0000-C000-000000000046}"))],
    ),
    k("Interface\\{B196B287-BAB4-101A-B69C-00AA00341D07}", &[("", Sz("IEnumConnections"))]),
    k(
        "Interface\\{B196B287-BAB4-101A-B69C-00AA00341D07}\\ProxyStubClsid32",
        &[("", Sz("{00000320-0000-0000-C000-000000000046}"))],
    ),
    k("Interface\\{6D5140C1-7436-11CE-8034-00AA006009FA}", &[("", Sz("IServiceProvider"))]),
    k(
        "Interface\\{6D5140C1-7436-11CE-8034-00AA006009FA}\\ProxyStubClsid32",
        &[("", Sz("{A6FF50C0-56C0-71CA-5732-BED303A59628}"))],
    ),
];

/// The proxy/stub and marshaller classes as the 64-bit view has them (`System32` paths).
pub const CLASSES_64: &[DefaultKey] = &[
    k("CLSID\\{0000002F-0000-0000-C000-000000000046}", &[("", Sz("CLSID_RecordInfo"))]),
    k(
        "CLSID\\{0000002F-0000-0000-C000-000000000046}\\InprocServer32",
        &[("", Sz("C:\\Windows\\System32\\oleaut32.dll")), ("ThreadingModel", Sz("Both"))],
    ),
    k("CLSID\\{00000300-0000-0000-C000-000000000046}", &[("", Sz("StdOleLink"))]),
    k("CLSID\\{00000300-0000-0000-C000-000000000046}\\InprocServer32", &[("", Sz("combase.dll"))]),
    k("CLSID\\{00000303-0000-0000-C000-000000000046}", &[("", Sz("FileMoniker"))]),
    k("CLSID\\{00000303-0000-0000-C000-000000000046}\\InprocServer32", &[("", Sz("combase.dll"))]),
    k("CLSID\\{00000303-0000-0000-C000-000000000046}\\ProgID", &[]),
    k("CLSID\\{00000304-0000-0000-C000-000000000046}", &[("", Sz("ItemMoniker"))]),
    k("CLSID\\{00000304-0000-0000-C000-000000000046}\\InprocServer32", &[("", Sz("combase.dll"))]),
    k("CLSID\\{00000305-0000-0000-C000-000000000046}", &[("", Sz("AntiMoniker"))]),
    k("CLSID\\{00000305-0000-0000-C000-000000000046}\\InprocServer32", &[("", Sz("combase.dll"))]),
    k("CLSID\\{00000306-0000-0000-C000-000000000046}", &[("", Sz("PointerMoniker"))]),
    k("CLSID\\{00000306-0000-0000-C000-000000000046}\\InprocServer32", &[("", Sz("combase.dll"))]),
    k("CLSID\\{00000308-0000-0000-C000-000000000046}", &[("", Sz("PackagerMoniker"))]),
    k("CLSID\\{00000308-0000-0000-C000-000000000046}\\InprocServer32", &[("", Sz("combase.dll"))]),
    k("CLSID\\{00000309-0000-0000-C000-000000000046}", &[("", Sz("CompositeMoniker"))]),
    k("CLSID\\{00000309-0000-0000-C000-000000000046}\\InprocServer32", &[("", Sz("combase.dll"))]),
    k("CLSID\\{0000030B-0000-0000-C000-000000000046}", &[("", Sz("DfMarshal"))]),
    k("CLSID\\{0000030B-0000-0000-C000-000000000046}\\InprocServer32", &[("", Sz("coml2.dll"))]),
    k("CLSID\\{00000315-0000-0000-C000-000000000046}", &[("", Sz("Picture (Metafile)"))]),
    k("CLSID\\{00000315-0000-0000-C000-000000000046}\\AuxUserType", &[]),
    k("CLSID\\{00000315-0000-0000-C000-000000000046}\\Conversion", &[]),
    k("CLSID\\{00000315-0000-0000-C000-000000000046}\\DataFormats", &[]),
    k("CLSID\\{00000315-0000-0000-C000-000000000046}\\InprocServer32", &[("", Sz("combase.dll"))]),
    k("CLSID\\{00000315-0000-0000-C000-000000000046}\\MiscStatus", &[]),
    k("CLSID\\{00000315-0000-0000-C000-000000000046}\\ProgID", &[]),
    k("CLSID\\{00000316-0000-0000-C000-000000000046}", &[("", Sz("Picture (Device Independent Bitmap)"))]),
    k("CLSID\\{00000316-0000-0000-C000-000000000046}\\AuxUserType", &[]),
    k("CLSID\\{00000316-0000-0000-C000-000000000046}\\Conversion", &[]),
    k("CLSID\\{00000316-0000-0000-C000-000000000046}\\DataFormats", &[]),
    k("CLSID\\{00000316-0000-0000-C000-000000000046}\\InprocServer32", &[("", Sz("combase.dll"))]),
    k("CLSID\\{00000316-0000-0000-C000-000000000046}\\MiscStatus", &[]),
    k("CLSID\\{00000316-0000-0000-C000-000000000046}\\ProgID", &[]),
    k("CLSID\\{00000319-0000-0000-C000-000000000046}", &[("", Sz("Picture (Enhanced Metafile)"))]),
    k("CLSID\\{00000319-0000-0000-C000-000000000046}\\AuxUserType", &[]),
    k("CLSID\\{00000319-0000-0000-C000-000000000046}\\Conversion", &[]),
    k("CLSID\\{00000319-0000-0000-C000-000000000046}\\DataFormats", &[]),
    k("CLSID\\{00000319-0000-0000-C000-000000000046}\\InprocServer32", &[("", Sz("combase.dll"))]),
    k("CLSID\\{00000319-0000-0000-C000-000000000046}\\MiscStatus", &[]),
    k("CLSID\\{0000031A-0000-0000-C000-000000000046}", &[("", Sz("ClassMoniker"))]),
    k("CLSID\\{0000031A-0000-0000-C000-000000000046}\\InprocServer32", &[("", Sz("combase.dll"))]),
    k("CLSID\\{0000031A-0000-0000-C000-000000000046}\\ProgID", &[]),
    k("CLSID\\{0000031D-0000-0000-C000-000000000046}", &[("", Sz("DCOMAccessControl"))]),
    k("CLSID\\{0000031D-0000-0000-C000-000000000046}\\InProcServer32", &[("", Sz("combase.dll"))]),
    k("CLSID\\{00000320-0000-0000-C000-000000000046}", &[("", Sz("PSFactoryBuffer"))]),
    k(
        "CLSID\\{00000320-0000-0000-C000-000000000046}\\InprocServer32",
        &[("", Sz("combase.dll")), ("ThreadingModel", Sz("Both"))],
    ),
    k("CLSID\\{00000327-0000-0000-C000-000000000046}", &[("", Sz("objref"))]),
    k(
        "CLSID\\{00000327-0000-0000-C000-000000000046}\\InprocServer32",
        &[("", Sz("combase.dll")), ("ThreadingModel", Sz("Both"))],
    ),
    k("CLSID\\{00000327-0000-0000-C000-000000000046}\\ProgID", &[]),
    k("CLSID\\{0000032E-0000-0000-C000-000000000046}", &[("", Sz("PipePSFactory"))]),
    k("CLSID\\{0000032E-0000-0000-C000-000000000046}\\InprocServer32", &[("", Sz("combase.dll"))]),
    k("CLSID\\{00000355-0000-0000-C000-000000000046}", &[("", Sz("MetadataPSFactory"))]),
    k(
        "CLSID\\{00000355-0000-0000-C000-000000000046}\\InProcServer32",
        &[("", Sz("C:\\Windows\\System32\\combase.dll")), ("ThreadingModel", Sz("Both"))],
    ),
    k("CLSID\\{00020420-0000-0000-C000-000000000046}", &[("", Sz("PSDispatch"))]),
    k(
        "CLSID\\{00020420-0000-0000-C000-000000000046}\\InprocServer32",
        &[("", Sz("C:\\Windows\\System32\\oleaut32.dll")), ("ThreadingModel", Sz("Both"))],
    ),
    k("CLSID\\{00020421-0000-0000-C000-000000000046}", &[("", Sz("PSEnumVariant"))]),
    k(
        "CLSID\\{00020421-0000-0000-C000-000000000046}\\InprocServer32",
        &[("", Sz("C:\\Windows\\System32\\oleaut32.dll")), ("ThreadingModel", Sz("Both"))],
    ),
    k("CLSID\\{00020422-0000-0000-C000-000000000046}", &[("", Sz("PSTypeInfo"))]),
    k(
        "CLSID\\{00020422-0000-0000-C000-000000000046}\\InprocServer32",
        &[("", Sz("C:\\Windows\\System32\\oleaut32.dll")), ("ThreadingModel", Sz("Both"))],
    ),
    k("CLSID\\{00020423-0000-0000-C000-000000000046}", &[("", Sz("PSTypeLib"))]),
    k(
        "CLSID\\{00020423-0000-0000-C000-000000000046}\\InprocServer32",
        &[("", Sz("C:\\Windows\\System32\\oleaut32.dll")), ("ThreadingModel", Sz("Both"))],
    ),
    k("CLSID\\{00020424-0000-0000-C000-000000000046}", &[("", Sz("PSOAInterface"))]),
    k(
        "CLSID\\{00020424-0000-0000-C000-000000000046}\\InprocServer32",
        &[("", Sz("C:\\Windows\\System32\\oleaut32.dll")), ("ThreadingModel", Sz("Both"))],
    ),
    k("CLSID\\{00020425-0000-0000-C000-000000000046}", &[("", Sz("PSTypeComp"))]),
    k(
        "CLSID\\{00020425-0000-0000-C000-000000000046}\\InprocServer32",
        &[("", Sz("C:\\Windows\\System32\\oleaut32.dll")), ("ThreadingModel", Sz("Both"))],
    ),
    k("CLSID\\{DF0B3D60-548F-101B-8E65-08002B2BD119}", &[("", Sz("PSSupportErrorInfo"))]),
    k(
        "CLSID\\{DF0B3D60-548F-101B-8E65-08002B2BD119}\\InprocServer32",
        &[("", Sz("C:\\Windows\\System32\\oleaut32.dll"))],
    ),
    k("CLSID\\{B196B286-BAB4-101A-B69C-00AA00341D07}", &[("", Sz("PSFactoryBuffer"))]),
    k(
        "CLSID\\{B196B286-BAB4-101A-B69C-00AA00341D07}\\InprocServer32",
        &[("", Sz("C:\\Windows\\System32\\oleaut32.dll")), ("ThreadingModel", Sz("Both"))],
    ),
    k("CLSID\\{4590F811-1D3A-11D0-891F-00AA004B2E24}", &[("", Sz("WBEM Locator"))]),
    k(
        "CLSID\\{4590F811-1D3A-11D0-891F-00AA004B2E24}\\InprocServer32",
        &[("", ExpandSz("%systemroot%\\system32\\wbem\\wbemprox.dll")), ("ThreadingModel", Sz("Both"))],
    ),
];

/// The same classes as the 32-bit view has them (`SysWOW64` for the absolute paths).
pub const CLASSES_32: &[DefaultKey] = &[
    k("CLSID\\{0000002F-0000-0000-C000-000000000046}", &[("", Sz("CLSID_RecordInfo"))]),
    k(
        "CLSID\\{0000002F-0000-0000-C000-000000000046}\\InprocServer32",
        &[("", Sz("C:\\Windows\\SysWOW64\\oleaut32.dll")), ("ThreadingModel", Sz("Both"))],
    ),
    k("CLSID\\{00000300-0000-0000-C000-000000000046}", &[("", Sz("StdOleLink"))]),
    k("CLSID\\{00000300-0000-0000-C000-000000000046}\\InprocServer32", &[("", Sz("combase.dll"))]),
    k("CLSID\\{00000303-0000-0000-C000-000000000046}", &[("", Sz("FileMoniker"))]),
    k("CLSID\\{00000303-0000-0000-C000-000000000046}\\InprocServer32", &[("", Sz("combase.dll"))]),
    k("CLSID\\{00000303-0000-0000-C000-000000000046}\\ProgID", &[]),
    k("CLSID\\{00000304-0000-0000-C000-000000000046}", &[("", Sz("ItemMoniker"))]),
    k("CLSID\\{00000304-0000-0000-C000-000000000046}\\InprocServer32", &[("", Sz("combase.dll"))]),
    k("CLSID\\{00000305-0000-0000-C000-000000000046}", &[("", Sz("AntiMoniker"))]),
    k("CLSID\\{00000305-0000-0000-C000-000000000046}\\InprocServer32", &[("", Sz("combase.dll"))]),
    k("CLSID\\{00000306-0000-0000-C000-000000000046}", &[("", Sz("PointerMoniker"))]),
    k("CLSID\\{00000306-0000-0000-C000-000000000046}\\InprocServer32", &[("", Sz("combase.dll"))]),
    k("CLSID\\{00000308-0000-0000-C000-000000000046}", &[("", Sz("PackagerMoniker"))]),
    k("CLSID\\{00000308-0000-0000-C000-000000000046}\\InprocServer32", &[("", Sz("combase.dll"))]),
    k("CLSID\\{00000309-0000-0000-C000-000000000046}", &[("", Sz("CompositeMoniker"))]),
    k("CLSID\\{00000309-0000-0000-C000-000000000046}\\InprocServer32", &[("", Sz("combase.dll"))]),
    k("CLSID\\{0000030B-0000-0000-C000-000000000046}", &[("", Sz("DfMarshal"))]),
    k("CLSID\\{0000030B-0000-0000-C000-000000000046}\\InprocServer32", &[("", Sz("coml2.dll"))]),
    k("CLSID\\{00000315-0000-0000-C000-000000000046}", &[("", Sz("Picture (Metafile)"))]),
    k("CLSID\\{00000315-0000-0000-C000-000000000046}\\AuxUserType", &[]),
    k("CLSID\\{00000315-0000-0000-C000-000000000046}\\Conversion", &[]),
    k("CLSID\\{00000315-0000-0000-C000-000000000046}\\DataFormats", &[]),
    k("CLSID\\{00000315-0000-0000-C000-000000000046}\\InprocServer32", &[("", Sz("combase.dll"))]),
    k("CLSID\\{00000315-0000-0000-C000-000000000046}\\MiscStatus", &[]),
    k("CLSID\\{00000315-0000-0000-C000-000000000046}\\ProgID", &[]),
    k("CLSID\\{00000316-0000-0000-C000-000000000046}", &[("", Sz("Picture (Device Independent Bitmap)"))]),
    k("CLSID\\{00000316-0000-0000-C000-000000000046}\\AuxUserType", &[]),
    k("CLSID\\{00000316-0000-0000-C000-000000000046}\\Conversion", &[]),
    k("CLSID\\{00000316-0000-0000-C000-000000000046}\\DataFormats", &[]),
    k("CLSID\\{00000316-0000-0000-C000-000000000046}\\InprocServer32", &[("", Sz("combase.dll"))]),
    k("CLSID\\{00000316-0000-0000-C000-000000000046}\\MiscStatus", &[]),
    k("CLSID\\{00000316-0000-0000-C000-000000000046}\\ProgID", &[]),
    k("CLSID\\{00000319-0000-0000-C000-000000000046}", &[("", Sz("Picture (Enhanced Metafile)"))]),
    k("CLSID\\{00000319-0000-0000-C000-000000000046}\\AuxUserType", &[]),
    k("CLSID\\{00000319-0000-0000-C000-000000000046}\\Conversion", &[]),
    k("CLSID\\{00000319-0000-0000-C000-000000000046}\\DataFormats", &[]),
    k("CLSID\\{00000319-0000-0000-C000-000000000046}\\InprocServer32", &[("", Sz("combase.dll"))]),
    k("CLSID\\{00000319-0000-0000-C000-000000000046}\\MiscStatus", &[]),
    k("CLSID\\{0000031A-0000-0000-C000-000000000046}", &[("", Sz("ClassMoniker"))]),
    k("CLSID\\{0000031A-0000-0000-C000-000000000046}\\InprocServer32", &[("", Sz("combase.dll"))]),
    k("CLSID\\{0000031A-0000-0000-C000-000000000046}\\ProgID", &[]),
    k("CLSID\\{0000031D-0000-0000-C000-000000000046}", &[("", Sz("DCOMAccessControl"))]),
    k("CLSID\\{0000031D-0000-0000-C000-000000000046}\\InProcServer32", &[("", Sz("combase.dll"))]),
    k("CLSID\\{00000320-0000-0000-C000-000000000046}", &[("", Sz("PSFactoryBuffer"))]),
    k(
        "CLSID\\{00000320-0000-0000-C000-000000000046}\\InprocServer32",
        &[("", Sz("combase.dll")), ("ThreadingModel", Sz("Both"))],
    ),
    k("CLSID\\{00000327-0000-0000-C000-000000000046}", &[("", Sz("objref"))]),
    k(
        "CLSID\\{00000327-0000-0000-C000-000000000046}\\InprocServer32",
        &[("", Sz("combase.dll")), ("ThreadingModel", Sz("Both"))],
    ),
    k("CLSID\\{00000327-0000-0000-C000-000000000046}\\ProgID", &[]),
    k("CLSID\\{0000032E-0000-0000-C000-000000000046}", &[("", Sz("PipePSFactory"))]),
    k("CLSID\\{0000032E-0000-0000-C000-000000000046}\\InprocServer32", &[("", Sz("combase.dll"))]),
    k("CLSID\\{00000355-0000-0000-C000-000000000046}", &[("", Sz("MetadataPSFactory"))]),
    k(
        "CLSID\\{00000355-0000-0000-C000-000000000046}\\InProcServer32",
        &[("", Sz("C:\\Windows\\SysWOW64\\combase.dll")), ("ThreadingModel", Sz("Both"))],
    ),
    k("CLSID\\{00020420-0000-0000-C000-000000000046}", &[("", Sz("PSDispatch"))]),
    k(
        "CLSID\\{00020420-0000-0000-C000-000000000046}\\InprocServer32",
        &[("", Sz("C:\\Windows\\SysWOW64\\oleaut32.dll")), ("ThreadingModel", Sz("Both"))],
    ),
    k("CLSID\\{00020421-0000-0000-C000-000000000046}", &[("", Sz("PSEnumVariant"))]),
    k(
        "CLSID\\{00020421-0000-0000-C000-000000000046}\\InprocServer32",
        &[("", Sz("C:\\Windows\\SysWOW64\\oleaut32.dll")), ("ThreadingModel", Sz("Both"))],
    ),
    k("CLSID\\{00020422-0000-0000-C000-000000000046}", &[("", Sz("PSTypeInfo"))]),
    k(
        "CLSID\\{00020422-0000-0000-C000-000000000046}\\InprocServer32",
        &[("", Sz("C:\\Windows\\SysWOW64\\oleaut32.dll")), ("ThreadingModel", Sz("Both"))],
    ),
    k("CLSID\\{00020423-0000-0000-C000-000000000046}", &[("", Sz("PSTypeLib"))]),
    k(
        "CLSID\\{00020423-0000-0000-C000-000000000046}\\InprocServer32",
        &[("", Sz("C:\\Windows\\SysWOW64\\oleaut32.dll")), ("ThreadingModel", Sz("Both"))],
    ),
    k("CLSID\\{00020424-0000-0000-C000-000000000046}", &[("", Sz("PSOAInterface"))]),
    k(
        "CLSID\\{00020424-0000-0000-C000-000000000046}\\InprocServer32",
        &[("", Sz("C:\\Windows\\SysWOW64\\oleaut32.dll")), ("ThreadingModel", Sz("Both"))],
    ),
    k("CLSID\\{00020425-0000-0000-C000-000000000046}", &[("", Sz("PSTypeComp"))]),
    k(
        "CLSID\\{00020425-0000-0000-C000-000000000046}\\InprocServer32",
        &[("", Sz("C:\\Windows\\SysWOW64\\oleaut32.dll")), ("ThreadingModel", Sz("Both"))],
    ),
    k("CLSID\\{DF0B3D60-548F-101B-8E65-08002B2BD119}", &[("", Sz("PSSupportErrorInfo"))]),
    k(
        "CLSID\\{DF0B3D60-548F-101B-8E65-08002B2BD119}\\InprocServer32",
        &[("", Sz("C:\\Windows\\SysWOW64\\oleaut32.dll"))],
    ),
    k("CLSID\\{B196B286-BAB4-101A-B69C-00AA00341D07}", &[("", Sz("PSFactoryBuffer"))]),
    k(
        "CLSID\\{B196B286-BAB4-101A-B69C-00AA00341D07}\\InprocServer32",
        &[("", Sz("C:\\Windows\\SysWOW64\\oleaut32.dll")), ("ThreadingModel", Sz("Both"))],
    ),
    k("CLSID\\{4590F811-1D3A-11D0-891F-00AA004B2E24}", &[("", Sz("WBEM Locator"))]),
    k(
        "CLSID\\{4590F811-1D3A-11D0-891F-00AA004B2E24}\\InprocServer32",
        &[("", ExpandSz("%systemroot%\\system32\\wbem\\wbemprox.dll")), ("ThreadingModel", Sz("Both"))],
    ),
];

/// The type library `stdole` (one key for both views: `win32` and `win64` paths).
pub const STDOLE: &[DefaultKey] = &[
    k("TypeLib\\{00020430-0000-0000-C000-000000000046}", &[]),
    k("TypeLib\\{00020430-0000-0000-C000-000000000046}\\1.0", &[("", Sz("OLE Automation"))]),
    k(
        "TypeLib\\{00020430-0000-0000-C000-000000000046}\\2.0",
        &[
            ("", Sz("OLE Automation")),
            (
                "PrimaryInteropAssemblyName",
                Sz("stdole, Version=7.0.3300.0, Culture=neutral, PublicKeyToken=b03f5f7f11d50a3a"),
            ),
        ],
    ),
    k("TypeLib\\{00020430-0000-0000-C000-000000000046}\\1.0\\0", &[]),
    k("TypeLib\\{00020430-0000-0000-C000-000000000046}\\1.0\\FLAGS", &[]),
    k("TypeLib\\{00020430-0000-0000-C000-000000000046}\\1.0\\HELPDIR", &[]),
    k("TypeLib\\{00020430-0000-0000-C000-000000000046}\\2.0\\0", &[]),
    k("TypeLib\\{00020430-0000-0000-C000-000000000046}\\2.0\\FLAGS", &[("", Sz("0"))]),
    k("TypeLib\\{00020430-0000-0000-C000-000000000046}\\2.0\\HELPDIR", &[("", Sz("C:\\Windows\\System32"))]),
    k(
        "TypeLib\\{00020430-0000-0000-C000-000000000046}\\2.0\\0\\win32",
        &[("", Sz("C:\\Windows\\SysWOW64\\stdole2.tlb"))],
    ),
    k(
        "TypeLib\\{00020430-0000-0000-C000-000000000046}\\2.0\\0\\win64",
        &[("", Sz("C:\\Windows\\System32\\stdole2.tlb"))],
    ),
    k(
        "TypeLib\\{00020430-0000-0000-C000-000000000046}\\1.0\\0\\win32",
        &[("", Sz("C:\\Windows\\System32\\stdole32.tlb"))],
    ),
];
