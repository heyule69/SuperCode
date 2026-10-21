//! Shared input budgets. Base64 expands images before they reach the local bridge.
pub const MAX_ATTACHMENTS: usize = 12;
pub const IMAGE_LIMIT: u64 = 10 * 1024 * 1024;
pub const TOTAL_IMAGE_LIMIT: u64 = 20 * 1024 * 1024;
pub const TEXT_LIMIT: usize = 1024 * 1024;
// Allow padding for independently encoded images, even when all 12 are present.
pub const ENCODED_IMAGE_LIMIT: usize =
    (TOTAL_IMAGE_LIMIT as usize).div_ceil(3) * 4 + (MAX_ATTACHMENTS - 1) * 4;
pub const BRIDGE_REQUEST_LIMIT: usize = 32 * 1024 * 1024;
// Keep a bounded allowance for history, tool schemas and protocol framing too.
const _: () = assert!(BRIDGE_REQUEST_LIMIT >= ENCODED_IMAGE_LIMIT + TEXT_LIMIT + 4 * 1024 * 1024);
