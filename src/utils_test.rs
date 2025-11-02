#[cfg(test)]
mod tests {
    use crate::utils::*;

    #[test]
    fn test_increment_resource_version() {
        assert_eq!(increment_resource_version("").unwrap(), "1");
        assert_eq!(increment_resource_version("1").unwrap(), "2");
        assert_eq!(increment_resource_version("999").unwrap(), "1");
        assert_eq!(increment_resource_version("42").unwrap(), "43");
    }
}
