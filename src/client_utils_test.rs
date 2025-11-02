#[cfg(test)]
mod tests {
    use crate::client_utils::*;

    #[test]
    fn test_pluralize() {
        assert_eq!(pluralize("Pod"), "pods");
        assert_eq!(pluralize("Deployment"), "deployments");
        assert_eq!(pluralize("ReplicaSet"), "replicasets");
    }
}
