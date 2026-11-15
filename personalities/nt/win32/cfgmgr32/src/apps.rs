//! More names the Windows applications of the survey import from `cfgmgr32.dll` (ADR 0155, step 10), fail-only like the
//! ones of `lib.rs` (same failure values); generated from the import tables of the programs and the coverage map.

poly_win32_rt::fail_exports!(
    "cfgmgr32",
    value(0x34),
    [
        CM_Get_Child,
        CM_Get_DevNode_Registry_PropertyW,
        CM_Get_DevNode_Status,
        CM_Get_Device_ID_ListW,
        CM_Get_Device_ID_List_SizeW,
        CM_Get_Device_Interface_List_SizeW,
        CM_Get_Sibling,
        CM_Open_DevNode_Key,
        CM_Register_Notification,
        CM_Unregister_Notification,
    ]
);
