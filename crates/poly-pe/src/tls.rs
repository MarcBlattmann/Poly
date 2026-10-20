//! The TLS directory (spec "The .tls Section"). Its fields are virtual
//! addresses (with the image at its header's `ImageBase`), not RVAs.

use crate::headers::{Bits, Pe, alignment_field};
use crate::read::{ptr_at, u32_at};
use crate::view::{View, rva_add};
use crate::{Error, dir};

/// The TLS directory (`IMAGE_TLS_DIRECTORY32/64`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TlsDirectory {
    /// VA of the first byte of the TLS template.
    pub start_address_of_raw_data: u64,
    /// VA just past the template's last byte.
    pub end_address_of_raw_data: u64,
    /// VA of the 32-bit slot that receives the module's TLS index.
    pub address_of_index: u64,
    /// VA of the zero-terminated array of callback VAs; zero for none.
    pub address_of_callbacks: u64,
    /// Bytes of zeros after the template in each thread's copy.
    pub size_of_zero_fill: u32,
    /// Bits 20-23 hold an `IMAGE_SCN_ALIGN_*` alignment for each copy.
    pub characteristics: u32,
}

impl TlsDirectory {
    /// Size of the template: `EndAddressOfRawData - StartAddressOfRawData`.
    pub fn template_size(&self) -> Result<u32, Error> {
        self.end_address_of_raw_data
            .checked_sub(self.start_address_of_raw_data)
            .and_then(|n| u32::try_from(n).ok())
            .ok_or(Error::Malformed("TLS template ends before it starts"))
    }

    /// Bytes each thread's copy needs: template plus zero fill.
    pub fn total_size(&self) -> Result<u32, Error> {
        self.template_size()?.checked_add(self.size_of_zero_fill).ok_or(Error::Malformed("TLS size overflows"))
    }

    /// Alignment each thread's copy needs, if the image says.
    pub fn alignment(&self) -> Option<u32> {
        alignment_field(self.characteristics)
    }
}

/// Iterator over TLS callback VAs, up to the zero entry. It stops after the
/// first error (for example an array without a terminator).
#[derive(Clone, Debug)]
pub struct TlsCallbacks<'a> {
    view: View<'a>,
    bits: Bits,
    rva: u32,
    index: usize,
    done: bool,
}

impl Iterator for TlsCallbacks<'_> {
    type Item = Result<u64, Error>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.done {
            return None;
        }
        let r = rva_add(self.rva, self.index, self.bits.pointer_size())
            .and_then(|rva| self.view.ptr(rva, self.bits.wide()));
        self.index += 1;
        match r {
            Ok(0) => {
                self.done = true;
                None
            }
            Ok(va) => Some(Ok(va)),
            Err(e) => {
                self.done = true;
                Some(Err(e))
            }
        }
    }
}

impl<'a> Pe<'a> {
    /// The TLS directory, if the image has one.
    pub fn tls(&self) -> Result<Option<TlsDirectory>, Error> {
        let Some(d) = self.info().directory(dir::TLS) else { return Ok(None) };
        let bits = self.info().bits();
        let p = bits.pointer_size();
        let b = self.view.slice(d.virtual_address, 4 * p + 8)?;
        let ptr = |i: usize| ptr_at(b, i * p, bits.wide()).unwrap_or(0);
        Ok(Some(TlsDirectory {
            start_address_of_raw_data: ptr(0),
            end_address_of_raw_data: ptr(1),
            address_of_index: ptr(2),
            address_of_callbacks: ptr(3),
            size_of_zero_fill: u32_at(b, 4 * p).unwrap_or(0),
            characteristics: u32_at(b, 4 * p + 4).unwrap_or(0),
        }))
    }

    fn va_to_rva(&self, va: u64) -> Result<u32, Error> {
        self.info().va_to_rva(va).ok_or(Error::BadVa(va))
    }

    /// The TLS template: the bytes each thread's copy starts with.
    pub fn tls_template(&self, tls: &TlsDirectory) -> Result<&'a [u8], Error> {
        let size = tls.template_size()?;
        if size == 0 {
            return Ok(&[]);
        }
        self.view.slice(self.va_to_rva(tls.start_address_of_raw_data)?, size as usize)
    }

    /// RVA of the slot that receives the module's TLS index.
    pub fn tls_index_rva(&self, tls: &TlsDirectory) -> Result<u32, Error> {
        self.va_to_rva(tls.address_of_index)
    }

    /// The TLS callbacks, as VAs with the image at its header's base. Call
    /// them in order (with `DLL_PROCESS_ATTACH` before the entry point).
    pub fn tls_callbacks(&self, tls: &TlsDirectory) -> Result<TlsCallbacks<'a>, Error> {
        let bits = self.info().bits();
        if tls.address_of_callbacks == 0 {
            return Ok(TlsCallbacks { view: self.view, bits, rva: 0, index: 0, done: true });
        }
        let rva = self.va_to_rva(tls.address_of_callbacks)?;
        Ok(TlsCallbacks { view: self.view, bits, rva, index: 0, done: false })
    }
}
