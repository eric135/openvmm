// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

//! Disk backend implementation that uses a user-mode storvsc driver.

#![forbid(unsafe_code)]

use disk_backend::DiskError;
use disk_backend::DiskIo;
use disk_backend::UnmapBehavior;
use inspect::Inspect;
use scsi_defs::ScsiOp;
use std::sync::Arc;
use storvsc_driver::StorvscDriver;
use vmbus_user_channel::MappedRingMem;
use zerocopy::FromBytes;
use zerocopy::FromZeros;
use zerocopy::IntoBytes;

/// Disk backend using a storvsc driver to the host.
#[derive(Inspect)]
pub struct StorvscDisk {
    #[inspect(skip)]
    driver: Arc<StorvscDriver<MappedRingMem>>,
    lun: u8,
    num_sectors: u64,
    sector_size: u32,
}

impl StorvscDisk {
    /// Creates a new storvsc-backed disk that uses the provided storvsc driver.
    pub fn new(driver: Arc<StorvscDriver<MappedRingMem>>, lun: u8) -> Self {
        let mut disk = Self {
            driver,
            lun,
            num_sectors: 0,
            sector_size: 0,
        };
        disk.scan_metadata();
        disk
    }
}

impl StorvscDisk {
    fn scan_metadata(&mut self) {
        // READ_CAPACITY16 returns number of sectors and sector size in bytes.
        let read_capacity16_cdb = scsi_defs::Cdb16 {
            operation_code: ScsiOp::READ_CAPACITY16,
            ..FromZeros::new_zeroed()
        };
        let request = self.generate_scsi_request(0, read_capacity16_cdb.as_bytes(), false);
        match futures::executor::block_on(self.driver.send_request(&request, 0, 0)) {
            Ok(resp) => {
                match scsi_defs::ReadCapacity16Data::read_from_prefix(resp.payload.as_bytes())
                    .map_err(|err| err.to_string())
                {
                    Ok(capacity) => {
                        self.num_sectors = capacity.0.ex.logical_block_address.into();
                        self.sector_size = capacity.0.ex.bytes_per_block.into();
                    }
                    Err(err) => {
                        tracing::error!(err, "READ_CAPACITY16 data parsing failed");
                    }
                }
            }
            Err(err) => {
                tracing::error!(
                    error = &err as &dyn std::error::Error,
                    "READ_CAPACITY16 failed"
                );
            }
        }
    }

    fn generate_scsi_request(
        &self,
        data_transfer_length: u32,
        payload: &[u8],
        is_read: bool,
    ) -> storvsp_protocol::ScsiRequest {
        assert!(payload.len() <= storvsp_protocol::MAX_DATA_BUFFER_LENGTH_WITH_PADDING);
        let data_in: u8 = if is_read { 1 } else { 0 };
        let mut request = storvsp_protocol::ScsiRequest {
            target_id: 0,
            path_id: 0,
            lun: self.lun,
            length: storvsp_protocol::SCSI_REQUEST_LEN_V2 as u16,
            cdb_length: payload.len() as u8,
            data_transfer_length,
            data_in,
            ..FromZeros::new_zeroed()
        };
        request.payload[0..payload.len()].copy_from_slice(payload);
        request
    }
}

impl DiskIo for StorvscDisk {
    fn disk_type(&self) -> &str {
        "storvsc"
    }

    fn sector_count(&self) -> u64 {
        self.num_sectors
    }

    fn sector_size(&self) -> u32 {
        self.sector_size
    }

    fn disk_id(&self) -> Option<[u8; 16]> {
        todo!()
    }

    fn physical_sector_size(&self) -> u32 {
        self.sector_size
    }

    fn is_fua_respected(&self) -> bool {
        // TODO
        false
    }

    fn is_read_only(&self) -> bool {
        // TODO
        false
    }

    async fn read_vectored(
        &self,
        buffers: &scsi_buffers::RequestBuffers<'_>,
        sector: u64,
    ) -> Result<(), DiskError> {
        if self.sector_size == 0 {
            // Disk failed to initialize.
            return Err(DiskError::IllegalBlock);
        }

        if buffers.len() % self.sector_size as usize != 0 {
            // Buffer length must be a multiple of sector size.
            return Err(DiskError::InvalidInput);
        }

        let cdb = scsi_defs::Cdb16 {
            operation_code: ScsiOp::READ16,
            logical_block: sector.into(),
            transfer_blocks: (buffers.len() as u32 / self.sector_size as u32).into(),
            ..FromZeros::new_zeroed()
        };
        let request = self.generate_scsi_request(0, cdb.as_bytes(), false);
        match self
            .driver
            .send_request(&request, buffers.guest_memory().inner_buf_mut()., 0)
            .await
        {
            Ok(resp) => {
                match scsi_defs::ReadCapacity16Data::read_from_prefix(resp.payload.as_bytes())
                    .map_err(|err| err.to_string())
                {
                    Ok(capacity) => capacity.0.ex.bytes_per_block.into(),
                    Err(err) => {
                        tracing::error!(err, "READ_CAPACITY16 data parsing failed");
                        0
                    }
                }
            }
            Err(err) => {
                tracing::error!(
                    error = &err as &dyn std::error::Error,
                    "READ_CAPACITY16 failed"
                );
                0
            }
        }
    }

    async fn write_vectored(
        &self,
        buffers: &scsi_buffers::RequestBuffers<'_>,
        sector: u64,
        fua: bool,
    ) -> Result<(), DiskError> {
        todo!()
    }

    async fn sync_cache(&self) -> Result<(), DiskError> {
        // SYNCHRONIZE_CACHE
        todo!()
    }

    async fn unmap(
        &self,
        sector: u64,
        count: u64,
        block_level_only: bool,
    ) -> Result<(), DiskError> {
        // UNMAP
        todo!()
    }

    fn unmap_behavior(&self) -> UnmapBehavior {
        todo!()
    }

    async fn wait_resize(&self, sector_count: u64) -> u64 {
        todo!();
    }
}
