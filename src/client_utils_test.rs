#[cfg(test)]
mod tests {
    use crate::client_utils::*;

    #[test]
    fn test_pluralize() {
        assert_eq!(pluralize("Pod"), "pods");
        assert_eq!(pluralize("Deployment"), "deployments");
        assert_eq!(pluralize("ReplicaSet"), "replicasets");
    }

    /// Test cases extracted from kube-rs to verify compatibility with native Kubernetes kinds
    /// Source: https://github.com/kube-rs/kube/blob/main/kube-core/src/discovery.rs
    #[test]
    fn test_pluralize_native_kinds() {
        // Extracted from Kubernetes swagger.json (as tested in kube-rs)
        #[rustfmt::skip]
        let native_kinds = vec![
            ("APIService", "apiservices"),
            ("Binding", "bindings"),
            ("CertificateSigningRequest", "certificatesigningrequests"),
            ("ClusterRole", "clusterroles"), ("ClusterRoleBinding", "clusterrolebindings"),
            ("ComponentStatus", "componentstatuses"),
            ("ConfigMap", "configmaps"),
            ("ControllerRevision", "controllerrevisions"),
            ("CronJob", "cronjobs"),
            ("CSIDriver", "csidrivers"), ("CSINode", "csinodes"), ("CSIStorageCapacity", "csistoragecapacities"),
            ("CustomResourceDefinition", "customresourcedefinitions"),
            ("DaemonSet", "daemonsets"),
            ("Deployment", "deployments"),
            ("Endpoints", "endpoints"), ("EndpointSlice", "endpointslices"),
            ("Event", "events"),
            ("FlowSchema", "flowschemas"),
            ("HorizontalPodAutoscaler", "horizontalpodautoscalers"),
            ("Ingress", "ingresses"), ("IngressClass", "ingressclasses"),
            ("Job", "jobs"),
            ("Lease", "leases"),
            ("LimitRange", "limitranges"),
            ("LocalSubjectAccessReview", "localsubjectaccessreviews"),
            ("MutatingWebhookConfiguration", "mutatingwebhookconfigurations"),
            ("Namespace", "namespaces"),
            ("NetworkPolicy", "networkpolicies"),
            ("Node", "nodes"),
            ("PersistentVolumeClaim", "persistentvolumeclaims"),
            ("PersistentVolume", "persistentvolumes"),
            ("PodDisruptionBudget", "poddisruptionbudgets"),
            ("Pod", "pods"),
            ("PodSecurityPolicy", "podsecuritypolicies"),
            ("PodTemplate", "podtemplates"),
            ("PriorityClass", "priorityclasses"),
            ("PriorityLevelConfiguration", "prioritylevelconfigurations"),
            ("ReplicaSet", "replicasets"),
            ("ReplicationController", "replicationcontrollers"),
            ("ResourceQuota", "resourcequotas"),
            ("Role", "roles"), ("RoleBinding", "rolebindings"),
            ("RuntimeClass", "runtimeclasses"),
            ("Secret", "secrets"),
            ("SelfSubjectAccessReview", "selfsubjectaccessreviews"),
            ("SelfSubjectRulesReview", "selfsubjectrulesreviews"),
            ("ServiceAccount", "serviceaccounts"),
            ("Service", "services"),
            ("StatefulSet", "statefulsets"),
            ("StorageClass", "storageclasses"), ("StorageVersion", "storageversions"),
            ("SubjectAccessReview", "subjectaccessreviews"),
            ("TokenReview", "tokenreviews"),
            ("ValidatingWebhookConfiguration", "validatingwebhookconfigurations"),
            ("VolumeAttachment", "volumeattachments"),
        ];

        for (kind, expected_plural) in native_kinds {
            assert_eq!(
                pluralize(kind),
                expected_plural,
                "Failed to pluralize {} correctly",
                kind
            );
        }
    }
}
