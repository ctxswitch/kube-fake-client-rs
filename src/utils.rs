use k8s_openapi::apimachinery::pkg::apis::meta::v1::ObjectMeta;

pub fn increment_generation(current: Option<i64>) -> i64 {
    current.unwrap_or(0) + 1
}

pub fn should_be_deleted(meta: &ObjectMeta) -> bool {
    meta.deletion_timestamp.is_some() && meta.finalizers.as_ref().is_none_or(Vec::is_empty)
}

pub fn ensure_metadata(meta: &mut ObjectMeta, namespace: &str) {
    // For cluster-scoped resources (empty namespace), ensure namespace is not set
    // For namespaced resources, set namespace if not present
    if namespace.is_empty() {
        meta.namespace = None;
    } else if meta.namespace.is_none() {
        meta.namespace = Some(namespace.to_string());
    }
    if meta.creation_timestamp.is_none() {
        meta.creation_timestamp = Some(k8s_openapi::apimachinery::pkg::apis::meta::v1::Time(
            chrono::Utc::now(),
        ));
    }
    if meta.uid.is_none() {
        meta.uid = Some(uuid::Uuid::new_v4().to_string());
    }
    if meta.generation.is_none() {
        meta.generation = Some(1);
    }
}

pub fn deletion_timestamp_equal(
    a: &Option<k8s_openapi::apimachinery::pkg::apis::meta::v1::Time>,
    b: &Option<k8s_openapi::apimachinery::pkg::apis::meta::v1::Time>,
) -> bool {
    match (a, b) {
        (Some(a), Some(b)) => a.0 == b.0,
        (None, None) => true,
        _ => false,
    }
}
