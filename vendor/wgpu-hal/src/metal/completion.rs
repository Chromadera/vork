//! Queue completion is a prefix of *all* native buffers, not just fence signals.
use alloc::{collections::VecDeque, string::String, sync::Arc};

use super::*;
use objc2::Message;

#[derive(Debug)]
struct Pending<T> {
    issued: u64,
    completed: u64,
    buffers: VecDeque<(u64, T)>,
}

impl<T> Default for Pending<T> {
    fn default() -> Self {
        Self {
            issued: 0,
            completed: 0,
            buffers: VecDeque::new(),
        }
    }
}

impl<T> Pending<T> {
    fn push(&mut self, buffer: T) -> u64 {
        self.issued += 1;
        self.buffers.push_back((self.issued, buffer));
        self.issued
    }

    fn poll(
        &mut self,
        mut status: impl FnMut(&T) -> Result<bool, crate::DeviceError>,
    ) -> Result<u64, crate::DeviceError> {
        let mut prefix = true;
        let mut completed = self.completed;
        // Inspect even buffers beyond an unfinished prefix: a later abort must
        // be reported promptly. Retire nothing until every status was checked.
        for (serial, buffer) in &self.buffers {
            prefix &= status(buffer)?;
            if prefix {
                completed = *serial;
            }
        }
        self.completed = completed;
        while self
            .buffers
            .front()
            .is_some_and(|(serial, _)| *serial <= completed)
        {
            self.buffers.pop_front();
        }
        Ok(completed)
    }
}

#[derive(Debug, Default)]
pub(super) struct QueueCompletion {
    pending: Mutex<Pending<Retained<ProtocolObject<dyn MTLCommandBuffer>>>>,
    // Callbacks own only this latch, never the pending buffers (no retain cycle).
    failure: Arc<Mutex<Option<String>>>,
}

// Metal command buffers permit concurrent status queries; access to the retained
// list is serialized by the mutex, as it was for HAL fences.
unsafe impl Send for QueueCompletion {}
unsafe impl Sync for QueueCompletion {}

fn failure(buffer: &ProtocolObject<dyn MTLCommandBuffer>) -> Option<String> {
    (buffer.status() == MTLCommandBufferStatus::Error).then(|| {
        format!(
            "Metal command buffer {:?} failed: {:?}",
            buffer.label(),
            buffer.error()
        )
    })
}

impl QueueCompletion {
    pub(super) fn track(&self, buffer: &ProtocolObject<dyn MTLCommandBuffer>) -> u64 {
        let latch = Arc::clone(&self.failure);
        let block = block2::RcBlock::new(
            move |buffer: NonNull<ProtocolObject<dyn MTLCommandBuffer>>| {
                if let Some(reason) = failure(unsafe { buffer.as_ref() }) {
                    latch.lock().get_or_insert(reason);
                }
            },
        );
        unsafe {
            buffer.addCompletedHandler(block2::RcBlock::as_ptr(&block));
        }
        self.pending.lock().push(buffer.retain())
    }

    pub(super) fn poll(&self) -> Result<u64, crate::DeviceError> {
        if let Some(reason) = self.failure.lock().as_ref() {
            return Err(crate::DeviceError::LostWithReason(reason.clone()));
        }
        let result = self.pending.lock().poll(|buffer| {
            // Metal can expose terminal status before invoking the handler.
            // Check it here too, before retiring buffers or publishing a fence.
            if let Some(reason) = failure(buffer) {
                return Err(crate::DeviceError::LostWithReason(reason));
            }
            Ok(buffer.status() == MTLCommandBufferStatus::Completed)
        });
        if let Err(crate::DeviceError::LostWithReason(reason)) = &result {
            self.failure.lock().get_or_insert_with(|| reason.clone());
        }
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn later_success_cannot_hide_earlier_error() {
        let mut pending = Pending::default();
        pending.push(0);
        pending.push(1);
        pending.push(2);
        let result = pending.poll(|&id| {
            if id == 1 {
                Err(crate::DeviceError::LostWithReason("native abort".into()))
            } else {
                Ok(true)
            }
        });
        assert!(
            matches!(result, Err(crate::DeviceError::LostWithReason(reason)) if reason == "native abort")
        );
        assert_eq!(pending.completed, 0);
        assert_eq!(pending.buffers.len(), 3);
    }

    #[test]
    fn completion_requires_the_entire_prefix() {
        let mut pending = Pending::default();
        pending.push(0);
        pending.push(1);
        pending.push(2);
        assert_eq!(pending.poll(|&id| Ok(id != 1)).unwrap(), 1);
        assert_eq!(pending.buffers.len(), 2);
        assert_eq!(pending.poll(|_| Ok(true)).unwrap(), 3);
        assert!(pending.buffers.is_empty());
        assert_eq!(pending.poll(|_| unreachable!()).unwrap(), 3);
    }

    #[test]
    fn error_beyond_pending_prefix_is_reported() {
        let mut pending = Pending::default();
        pending.push(false);
        pending.push(true);
        assert!(pending
            .poll(|&failed| if failed {
                Err(crate::DeviceError::LostWithReason("abort".into()))
            } else {
                Ok(false)
            })
            .is_err());
        assert_eq!(pending.completed, 0);
    }
}
