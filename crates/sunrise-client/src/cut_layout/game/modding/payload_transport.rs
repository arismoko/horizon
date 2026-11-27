#![cfg(test)]
#[cfg(test)]
#[path = "payload_transport/config_tests.rs"]
mod config_tests;
#[cfg(test)]
#[path = "payload_transport/disconnect_tests.rs"]
mod disconnect_tests;
#[cfg(test)]
#[path = "payload_transport/raw_fault_tests.rs"]
mod raw_fault_tests;
#[cfg(test)]
#[path = "payload_transport/sender_tests.rs"]
mod sender_tests;
#[cfg(test)]
#[path = "payload_transport/task_tests.rs"]
mod task_tests;
#[cfg(test)]
#[path = "payload_transport/tests.rs"]
mod tests;
#[cfg(test)]
#[path = "payload_transport/typed_tests.rs"]
mod typed_tests;
#[cfg(test)]
#[path = "payload_transport/virtual_host_tests.rs"]
mod virtual_host_tests;
#[cfg(test)]
#[path = "payload_transport/work_panic_tests.rs"]
mod work_panic_tests;
