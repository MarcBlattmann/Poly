//! The common configuration structure (§4.1.4.3) and device initialisation
//! (§3.1.1).
//!
//! Initialisation is a fixed sequence of device status values, and the types
//! here only allow it in that order:
//!
//! ```text
//! begin():           reset, wait for status 0, ACKNOWLEDGE, DRIVER, read features
//!   -> FeatureNegotiation::accept():  write features, FEATURES_OK, check it stuck
//!   -> Configuring:  set up virtqueues and interrupt vectors
//!   -> Configuring::driver_ok():      DRIVER_OK, the device is live
//! ```
//!
//! Every failure after the reset sets FAILED, except queue setup errors,
//! which the driver may work around (a smaller queue, no MSI-X vector)
//! before giving up with [`Configuring::fail`].

use crate::{QueueAddresses, feature, status};
use poly_pci::Mmio;

/// Register offsets (§4.1.4.3, `struct virtio_pci_common_cfg`).
mod reg {
    pub const DEVICE_FEATURE_SELECT: usize = 0x00;
    pub const DEVICE_FEATURE: usize = 0x04;
    pub const DRIVER_FEATURE_SELECT: usize = 0x08;
    pub const DRIVER_FEATURE: usize = 0x0c;
    pub const CONFIG_MSIX_VECTOR: usize = 0x10;
    pub const NUM_QUEUES: usize = 0x12;
    pub const DEVICE_STATUS: usize = 0x14;
    pub const CONFIG_GENERATION: usize = 0x15;
    pub const QUEUE_SELECT: usize = 0x16;
    pub const QUEUE_SIZE: usize = 0x18;
    pub const QUEUE_MSIX_VECTOR: usize = 0x1a;
    pub const QUEUE_ENABLE: usize = 0x1c;
    pub const QUEUE_NOTIFY_OFF: usize = 0x1e;
    pub const QUEUE_DESC: usize = 0x20;
    pub const QUEUE_DRIVER: usize = 0x28;
    pub const QUEUE_DEVICE: usize = 0x30;
}

/// Bytes of the common configuration structure this crate uses: the fields
/// virtio 1.0 defines. Later revisions append more.
pub const COMMON_CFG_MIN_LEN: u32 = 0x38;

/// "No MSI-X vector" (§4.1.5.1.2), also what the device reads back when it
/// could not map the vector asked for.
pub const NO_VECTOR: u16 = 0xffff;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InitError {
    /// The device status did not read 0 after a reset.
    ResetTimeout,
    /// The device does not offer VIRTIO_F_VERSION_1.
    LegacyOnly,
    /// The device cleared FEATURES_OK: it cannot work with the features the
    /// driver picked.
    FeaturesRejected,
    NoSuchQueue,
    /// The device reports size 0 for the queue.
    QueueUnavailable,
    QueueAlreadyEnabled,
    /// Not a power of two, or above the device's maximum.
    BadQueueSize,
    /// The device could not map the MSI-X vector.
    VectorRejected,
}

/// Typed access to the common configuration structure. `regs` maps it
/// (offset 0 is `device_feature_select`) and must be at least
/// [`COMMON_CFG_MIN_LEN`] bytes long; [`crate::PciTransport::discover`]
/// checks that for the region it reports.
#[derive(Debug)]
pub struct CommonCfg<M> {
    regs: M,
}

impl<M: Mmio> CommonCfg<M> {
    pub const fn new(regs: M) -> CommonCfg<M> {
        CommonCfg { regs }
    }

    pub const fn regs(&self) -> &M {
        &self.regs
    }

    pub fn status(&self) -> u8 {
        self.regs.read8(reg::DEVICE_STATUS)
    }

    pub fn set_status(&self, value: u8) {
        self.regs.write8(reg::DEVICE_STATUS, value);
    }

    /// Reset the device and wait, polling at most `max_polls` times, until
    /// it reports status 0 (§4.1.4.3.2: the driver must not touch the
    /// device again before that).
    pub fn reset(&self, max_polls: u32) -> Result<(), InitError> {
        self.set_status(0);
        for _ in 0..max_polls {
            if self.status() == 0 {
                return Ok(());
            }
            core::hint::spin_loop();
        }
        Err(InitError::ResetTimeout)
    }

    /// Features bits 0-63 the device offers.
    pub fn device_features(&self) -> u64 {
        self.regs.write32(reg::DEVICE_FEATURE_SELECT, 0);
        let lo = self.regs.read32(reg::DEVICE_FEATURE);
        self.regs.write32(reg::DEVICE_FEATURE_SELECT, 1);
        let hi = self.regs.read32(reg::DEVICE_FEATURE);
        (hi as u64) << 32 | lo as u64
    }

    pub fn set_driver_features(&self, features: u64) {
        self.regs.write32(reg::DRIVER_FEATURE_SELECT, 0);
        self.regs.write32(reg::DRIVER_FEATURE, features as u32);
        self.regs.write32(reg::DRIVER_FEATURE_SELECT, 1);
        self.regs.write32(reg::DRIVER_FEATURE, (features >> 32) as u32);
    }

    pub fn num_queues(&self) -> u16 {
        self.regs.read16(reg::NUM_QUEUES)
    }

    pub fn config_generation(&self) -> u8 {
        self.regs.read8(reg::CONFIG_GENERATION)
    }

    /// Run `read` (a read of the device configuration structure) until the
    /// configuration generation is the same before and after, at most
    /// `max_tries` times (§2.5.1): fields wider than 32 bits, or several
    /// fields together, are only consistent that way.
    pub fn read_config<T>(&self, max_tries: u32, mut read: impl FnMut() -> T) -> Option<T> {
        for _ in 0..max_tries {
            let before = self.config_generation();
            let value = read();
            if self.config_generation() == before {
                return Some(value);
            }
        }
        None
    }

    /// Route configuration change interrupts to MSI-X `vector` (or
    /// [`NO_VECTOR`]). Returns what the device read back.
    pub fn set_config_msix_vector(&self, vector: u16) -> u16 {
        self.regs.write16(reg::CONFIG_MSIX_VECTOR, vector);
        self.regs.read16(reg::CONFIG_MSIX_VECTOR)
    }

    /// Make queue `index` the one the `queue_*` registers refer to.
    pub fn select_queue(&self, index: u16) {
        self.regs.write16(reg::QUEUE_SELECT, index);
    }

    /// Of the selected queue: the maximum size until the driver writes a
    /// size, then that size. 0 if the queue does not exist.
    pub fn queue_size(&self) -> u16 {
        self.regs.read16(reg::QUEUE_SIZE)
    }

    pub fn set_queue_size(&self, size: u16) {
        self.regs.write16(reg::QUEUE_SIZE, size);
    }

    pub fn queue_enabled(&self) -> bool {
        self.regs.read16(reg::QUEUE_ENABLE) != 0
    }

    pub fn enable_selected_queue(&self) {
        self.regs.write16(reg::QUEUE_ENABLE, 1);
    }

    pub fn queue_notify_off(&self) -> u16 {
        self.regs.read16(reg::QUEUE_NOTIFY_OFF)
    }

    /// Route the selected queue's interrupts to MSI-X `vector` (or
    /// [`NO_VECTOR`]). Returns what the device read back.
    pub fn set_queue_msix_vector(&self, vector: u16) -> u16 {
        self.regs.write16(reg::QUEUE_MSIX_VECTOR, vector);
        self.regs.read16(reg::QUEUE_MSIX_VECTOR)
    }

    /// Program the selected queue's three addresses. The 64-bit fields are
    /// written as two 32-bit halves, low first, which §4.1.3.1 allows and
    /// every transport handles.
    pub fn set_queue_addresses(&self, a: QueueAddresses) {
        for (off, value) in [(reg::QUEUE_DESC, a.desc), (reg::QUEUE_DRIVER, a.driver), (reg::QUEUE_DEVICE, a.device)] {
            self.regs.write32(off, value as u32);
            self.regs.write32(off + 4, (value >> 32) as u32);
        }
    }
}

/// Start initialising a device (§3.1.1 steps 1-3): reset it, set
/// ACKNOWLEDGE and DRIVER, and read what it offers. Refuses devices without
/// VIRTIO_F_VERSION_1, marking them FAILED.
pub fn begin<M: Mmio>(common: &CommonCfg<M>, max_polls: u32) -> Result<FeatureNegotiation<'_, M>, InitError> {
    common.reset(max_polls)?;
    common.set_status(status::ACKNOWLEDGE);
    let driver_status = status::ACKNOWLEDGE | status::DRIVER;
    common.set_status(driver_status);
    let offered = common.device_features();
    if offered & feature::VERSION_1 == 0 {
        common.set_status(driver_status | status::FAILED);
        return Err(InitError::LegacyOnly);
    }
    Ok(FeatureNegotiation { common, offered, driver_status })
}

/// The device has been acknowledged and its features read.
#[derive(Debug)]
pub struct FeatureNegotiation<'a, M> {
    common: &'a CommonCfg<M>,
    offered: u64,
    /// Status bits the driver has set so far. Status writes always carry
    /// all of them: the driver must never clear one (§2.1.1).
    driver_status: u8,
}

impl<'a, M: Mmio> FeatureNegotiation<'a, M> {
    pub const fn offered(&self) -> u64 {
        self.offered
    }

    /// Accept the offered features that are also in `wanted`, plus
    /// VERSION_1, minus [`feature::UNSUPPORTED`] (§3.1.1 steps 4-6). Fails,
    /// marking the device FAILED, if the device then refuses FEATURES_OK.
    pub fn accept(self, wanted: u64) -> Result<Configuring<'a, M>, InitError> {
        let features = self.offered & (wanted | feature::VERSION_1) & !feature::UNSUPPORTED;
        self.common.set_driver_features(features);
        let driver_status = self.driver_status | status::FEATURES_OK;
        self.common.set_status(driver_status);
        if self.common.status() & status::FEATURES_OK == 0 {
            self.common.set_status(self.driver_status | status::FAILED);
            return Err(InitError::FeaturesRejected);
        }
        Ok(Configuring { common: self.common, features, driver_status })
    }

    /// Give up: mark the device FAILED.
    pub fn fail(self) {
        self.common.set_status(self.driver_status | status::FAILED);
    }
}

/// Features are agreed; the driver now sets up queues and vectors (§3.1.1
/// step 7) and then calls [`Configuring::driver_ok`].
#[derive(Debug)]
pub struct Configuring<'a, M> {
    common: &'a CommonCfg<M>,
    features: u64,
    driver_status: u8,
}

impl<M: Mmio> Configuring<'_, M> {
    /// The negotiated features.
    pub const fn features(&self) -> u64 {
        self.features
    }

    pub const fn has(&self, feature: u64) -> bool {
        self.features & feature == feature
    }

    pub const fn common(&self) -> &CommonCfg<M> {
        self.common
    }

    /// Largest size the device supports for queue `index`.
    pub fn queue_max_size(&self, index: u16) -> Result<u16, InitError> {
        if index >= self.common.num_queues() {
            return Err(InitError::NoSuchQueue);
        }
        self.common.select_queue(index);
        match self.common.queue_size() {
            0 => Err(InitError::QueueUnavailable),
            size => Ok(size),
        }
    }

    /// Route configuration change interrupts to MSI-X `vector`. MSI-X must
    /// be enabled in the PCI capability first, or the device ignores it.
    pub fn set_config_vector(&self, vector: u16) -> Result<(), InitError> {
        if self.common.set_config_msix_vector(vector) != vector {
            return Err(InitError::VectorRejected);
        }
        Ok(())
    }

    /// Set up and enable queue `index` (§4.1.5.1.3): size, MSI-X vector
    /// (`None` for none), the addresses from [`crate::SplitQueue::addresses`],
    /// then `queue_enable`. Returns the queue's `queue_notify_off`, for
    /// [`crate::PciTransport::notifier`].
    pub fn enable_queue(
        &self,
        index: u16,
        size: u16,
        addresses: QueueAddresses,
        vector: Option<u16>,
    ) -> Result<u16, InitError> {
        let max = self.queue_max_size(index)?;
        if self.common.queue_enabled() {
            return Err(InitError::QueueAlreadyEnabled);
        }
        if !size.is_power_of_two() || size > max {
            return Err(InitError::BadQueueSize);
        }
        self.common.set_queue_size(size);
        let vector = vector.unwrap_or(NO_VECTOR);
        if self.common.set_queue_msix_vector(vector) != vector {
            return Err(InitError::VectorRejected);
        }
        self.common.set_queue_addresses(addresses);
        let notify_off = self.common.queue_notify_off();
        self.common.enable_selected_queue();
        Ok(notify_off)
    }

    /// Finish initialisation (§3.1.1 step 8): the device may now use its
    /// queues and raise interrupts. Returns the negotiated features.
    pub fn driver_ok(self) -> u64 {
        self.common.set_status(self.driver_status | status::DRIVER_OK);
        self.features
    }

    /// Give up: mark the device FAILED.
    pub fn fail(self) {
        self.common.set_status(self.driver_status | status::FAILED);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sim::{CommonDevice, QueueRegs};
    use status::*;

    const OFFER: u64 = feature::VERSION_1 | feature::EVENT_IDX | feature::RING_PACKED | feature::IN_ORDER | 0b101;
    const ADDRS: QueueAddresses = QueueAddresses { desc: 0x1_0000_1000, driver: 0x1_0000_2000, device: 0x1_0000_3000 };

    fn device(offered: u64) -> CommonCfg<CommonDevice> {
        CommonCfg::new(CommonDevice::new(offered, &[256, 0, 64]))
    }

    #[test]
    fn full_initialisation() {
        let c = device(OFFER);
        c.regs().status.set(DRIVER_OK | ACKNOWLEDGE);
        c.regs().reset_delay.set(3);
        let n = begin(&c, 10).unwrap();
        assert_eq!(n.offered(), OFFER);
        let cfg = n.accept(feature::EVENT_IDX | feature::RING_PACKED | feature::IN_ORDER | 0b001).unwrap();
        // Packed rings and in-order use are never accepted; bit 2 was not
        // wanted; VERSION_1 always is.
        assert_eq!(cfg.features(), feature::VERSION_1 | feature::EVENT_IDX | 0b001);
        assert!(cfg.has(feature::EVENT_IDX));
        assert_eq!(c.regs().driver_features.get(), cfg.features());
        assert_eq!(cfg.queue_max_size(0), Ok(256));
        assert_eq!(cfg.enable_queue(2, 64, ADDRS, Some(1)), Ok(2));
        cfg.set_config_vector(0).unwrap();
        assert_eq!(cfg.driver_ok(), feature::VERSION_1 | feature::EVENT_IDX | 0b001);
        assert_eq!(*c.regs().status_writes.borrow(), [0, 1, 3, 11, 15]);
        assert_eq!(c.status(), 15);
        let q = c.regs().queues.borrow()[2];
        assert_eq!(
            q,
            QueueRegs {
                max_size: 64,
                size: 64,
                msix: 1,
                enable: 1,
                notify_off: 2,
                desc: ADDRS.desc,
                driver: ADDRS.driver,
                device: ADDRS.device
            }
        );
    }

    #[test]
    fn reset_timeout() {
        let c = device(OFFER);
        c.regs().reset_delay.set(5);
        assert_eq!(begin(&c, 5).err(), Some(InitError::ResetTimeout));
        assert_eq!(*c.regs().status_writes.borrow(), [0]);
        c.regs().reset_delay.set(4);
        assert!(begin(&c, 5).is_ok());
    }

    #[test]
    fn legacy_only_device_is_refused() {
        let c = device(feature::EVENT_IDX);
        assert_eq!(begin(&c, 1).err(), Some(InitError::LegacyOnly));
        assert_eq!(c.status(), ACKNOWLEDGE | DRIVER | FAILED);
    }

    #[test]
    fn rejected_features() {
        let mut d = CommonDevice::new(OFFER, &[8]);
        d.accepts = |f| f & feature::EVENT_IDX != 0;
        let c = CommonCfg::new(d);
        let err = begin(&c, 1).unwrap().accept(0).err();
        assert_eq!(err, Some(InitError::FeaturesRejected));
        assert_eq!(c.status(), ACKNOWLEDGE | DRIVER | FAILED);
        // Starting over with what the device wants works.
        let cfg = begin(&c, 1).unwrap().accept(feature::EVENT_IDX).unwrap();
        cfg.fail();
        assert_eq!(c.status(), ACKNOWLEDGE | DRIVER | FEATURES_OK | FAILED);
    }

    #[test]
    fn abandoning_negotiation() {
        let c = device(OFFER);
        begin(&c, 1).unwrap().fail();
        assert_eq!(c.status(), ACKNOWLEDGE | DRIVER | FAILED);
    }

    #[test]
    fn queue_setup_errors() {
        let c = device(OFFER);
        let cfg = begin(&c, 1).unwrap().accept(0).unwrap();
        assert_eq!(cfg.queue_max_size(3), Err(InitError::NoSuchQueue));
        assert_eq!(cfg.queue_max_size(1), Err(InitError::QueueUnavailable));
        assert_eq!(cfg.enable_queue(0, 512, ADDRS, None), Err(InitError::BadQueueSize));
        assert_eq!(cfg.enable_queue(0, 48, ADDRS, None), Err(InitError::BadQueueSize));
        assert_eq!(cfg.enable_queue(0, 0, ADDRS, None), Err(InitError::BadQueueSize));
        assert_eq!(cfg.enable_queue(0, 16, ADDRS, Some(4)), Err(InitError::VectorRejected));
        assert_eq!(cfg.set_config_vector(9), Err(InitError::VectorRejected));
        assert_eq!(cfg.enable_queue(0, 16, ADDRS, None), Ok(0));
        assert_eq!(c.regs().queues.borrow()[0].msix, NO_VECTOR);
        assert_eq!(cfg.enable_queue(0, 16, ADDRS, None), Err(InitError::QueueAlreadyEnabled));
    }

    #[test]
    fn reset_clears_queues() {
        let c = device(OFFER);
        let cfg = begin(&c, 1).unwrap().accept(0).unwrap();
        cfg.enable_queue(0, 16, ADDRS, None).unwrap();
        cfg.driver_ok();
        let cfg = begin(&c, 1).unwrap().accept(0).unwrap();
        assert_eq!(cfg.queue_max_size(0), Ok(256));
        assert_eq!(cfg.enable_queue(0, 16, ADDRS, None), Ok(0));
    }

    #[test]
    fn config_reads_retry_on_generation_change() {
        let c = device(OFFER);
        let mut reads = 0;
        // The generation moves during the first two attempts.
        c.regs().generation_churn.set(2);
        assert_eq!(
            c.read_config(5, || {
                reads += 1;
                reads
            }),
            Some(2)
        );
        c.regs().generation_churn.set(10);
        assert_eq!(c.read_config(3, || ()), None);
    }
}
