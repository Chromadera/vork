//! Invocation-local reference for the media shader work ledger.
//! One instance must flow through path, boundary, coefficient and shadow helpers.
//! This CPU contract is test-only until its shader counterpart is integrated.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Work {
    Boundary,
    Coefficient,
    Collision,
    Connection,
}

#[derive(Debug, PartialEq, Eq)]
struct Exhausted {
    operation: Work,
}

struct Ledger {
    remaining: u32,
    failed: Option<Work>,
}
impl Ledger {
    fn new(limit: u32) -> Self {
        Self { remaining: limit, failed: None }
    }

    /// Reserve before doing work. A rejected reservation never executes partly,
    /// and exhaustion remains sticky even if a caller catches the error.
    fn reserve(&mut self, operation: Work, cost: u32) -> Result<(), Exhausted> {
        if let Some(operation) = self.failed {
            return Err(Exhausted { operation });
        }
        match self.remaining.checked_sub(cost) {
            Some(remaining) => { self.remaining = remaining; Ok(()) }
            None => {
                self.failed = Some(operation);
                Err(Exhausted { operation })
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn connection(ledger: &mut Ledger) -> Result<(), Exhausted> {
        ledger.reserve(Work::Connection, 1)?;
        ledger.reserve(Work::Boundary, 32)?;
        ledger.reserve(Work::Coefficient, 8)
    }

    #[test]
    fn nested_connections_share_one_budget() {
        let mut ledger = Ledger::new(83);
        connection(&mut ledger).unwrap();
        connection(&mut ledger).unwrap();
        ledger.reserve(Work::Collision, 1).unwrap();
        assert_eq!(ledger.remaining, 0);
        assert_eq!(connection(&mut ledger), Err(Exhausted { operation: Work::Connection }));
        // Catching an error cannot turn the remaining frame into vacuum.
        assert_eq!(ledger.reserve(Work::Coefficient, 0), Err(Exhausted { operation: Work::Connection }));
    }

    #[test]
    fn oversized_reservation_never_wraps_or_spends_partial_credit() {
        let mut ledger = Ledger::new(3);
        assert_eq!(ledger.reserve(Work::Boundary, u32::MAX), Err(Exhausted { operation: Work::Boundary }));
        assert_eq!(ledger.remaining, 3);
        assert_eq!(ledger.failed, Some(Work::Boundary));
    }
}
