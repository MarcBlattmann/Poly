//! The functions the Windows applications of the survey (`windows-app-survey`) import from `winmm.dll` that Poly does
//! not implement yet (ADR 0155, step 10): each is an export that logs `stub: winmm!name` and fails as ADR 0155 fixes for a
//! function the system does not provide, so that the programs bind it (and `GetProcAddress` finds it). An
//! implementation replaces a name here by defining it elsewhere in the crate and deleting it from the list.
//! Generated from the programs' import tables (names only) and our export table.

poly_win32_rt::fail_exports!(
    "winmm",
    value(0),
    [PlaySoundA, PlaySoundW, midiInGetNumDevs, midiOutGetNumDevs, timeSetEvent, waveInGetNumDevs,]
);

poly_win32_rt::fail_exports!("winmm", value(277), [mciSendCommandA, mciSendCommandW,]);

poly_win32_rt::fail_exports!(
    "winmm",
    value(8),
    [
        midiInAddBuffer,
        midiInClose,
        midiInGetDevCapsW,
        midiInMessage,
        midiInOpen,
        midiInPrepareHeader,
        midiInReset,
        midiInStart,
        midiInStop,
        midiInUnprepareHeader,
        midiOutClose,
        midiOutGetDevCapsW,
        midiOutLongMsg,
        midiOutMessage,
        midiOutOpen,
        midiOutPrepareHeader,
        midiOutReset,
        midiOutShortMsg,
        midiOutUnprepareHeader,
        mixerClose,
        mixerGetControlDetailsW,
        mixerGetDevCapsW,
        mixerGetID,
        mixerGetLineControlsW,
        mixerGetLineInfoW,
        mixerOpen,
        mixerSetControlDetails,
        timeKillEvent,
        waveInAddBuffer,
        waveInClose,
        waveInGetDevCapsA,
        waveInGetDevCapsW,
        waveInGetErrorTextW,
        waveInMessage,
        waveInOpen,
        waveInPrepareHeader,
        waveInReset,
        waveInStart,
        waveInUnprepareHeader,
    ]
);

poly_win32_rt::fail_exports!("winmm", value(97), [timeGetDevCaps,]);
