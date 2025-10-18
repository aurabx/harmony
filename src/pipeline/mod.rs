pub mod executor;

#[cfg(test)]
mod tests;

// Re-exports for convenience
pub use executor::PipelineExecutor;
