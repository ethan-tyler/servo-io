# Workload Identity Migration Guide

This guide walks through migrating from service account JSON keys to **Workload Identity**, eliminating the need to manage long-lived credentials.

## Benefits of Workload Identity

✅ **No JSON key files** - Credentials are never stored in files or environment variables
✅ **Automatic key rotation** - Google manages credential lifecycle
✅ **Reduced attack surface** - No long-lived secrets to compromise
✅ **Better audit logging** - Clear attribution of API calls to workload
✅ **Simplified deployment** - No secret distribution/rotation workflows

---

## Prerequisites

- GKE cluster with Workload Identity enabled, OR
- Cloud Run service (Workload Identity is default)
- `gcloud` CLI installed and configured
- Owner or Security Admin role on GCP project

---

## Migration Steps

### 1. Create Google Service Account (if not exists)

```bash
# Set your project ID
export PROJECT_ID="your-gcp-project"
export GSA_NAME="servo-worker"

# Create service account
gcloud iam service-accounts create $GSA_NAME \
    --project=$PROJECT_ID \
    --display-name="Servo Worker Service Account" \
    --description="Service account for Servo workflow execution"
```

### 2. Grant Required Permissions

```bash
# Grant permissions for Cloud Tasks
gcloud projects add-iam-policy-binding $PROJECT_ID \
    --member="serviceAccount:${GSA_NAME}@${PROJECT_ID}.iam.gserviceaccount.com" \
    --role="roles/cloudtasks.enqueuer"

# Grant permissions for Secret Manager (for HMAC secrets)
gcloud projects add-iam-policy-binding $PROJECT_ID \
    --member="serviceAccount:${GSA_NAME}@${PROJECT_ID}.iam.gserviceaccount.com" \
    --role="roles/secretmanager.secretAccessor"

# If using Cloud Logging
gcloud projects add-iam-policy-binding $PROJECT_ID \
    --member="serviceAccount:${GSA_NAME}@${PROJECT_ID}.iam.gserviceaccount.com" \
    --role="roles/logging.logWriter"
```

### 3. Option A: Cloud Run (Recommended - Easiest)

Workload Identity is **enabled by default** on Cloud Run. Simply specify the service account when deploying:

```bash
gcloud run deploy servo-worker \
    --image=gcr.io/${PROJECT_ID}/servo-worker:latest \
    --service-account=${GSA_NAME}@${PROJECT_ID}.iam.gserviceaccount.com \
    --region=us-central1 \
    --platform=managed \
    --no-allow-unauthenticated
```

**That's it!** Your Cloud Run service now uses Workload Identity automatically.

### 3. Option B: GKE with Workload Identity

#### Enable Workload Identity on GKE Cluster

```bash
export CLUSTER_NAME="servo-cluster"
export REGION="us-central1"

# Enable Workload Identity on existing cluster (if not already enabled)
gcloud container clusters update $CLUSTER_NAME \
    --region=$REGION \
    --workload-pool=${PROJECT_ID}.svc.id.goog
```

#### Create Kubernetes Service Account

```yaml
# k8s-service-account.yaml
apiVersion: v1
kind: ServiceAccount
metadata:
  name: servo-worker
  namespace: default
```

```bash
kubectl apply -f k8s-service-account.yaml
```

#### Bind Kubernetes SA to Google SA

```bash
export KSA_NAME="servo-worker"
export NAMESPACE="default"

# Allow Kubernetes SA to impersonate Google SA
gcloud iam service-accounts add-iam-policy-binding \
    ${GSA_NAME}@${PROJECT_ID}.iam.gserviceaccount.com \
    --role=roles/iam.workloadIdentityUser \
    --member="serviceAccount:${PROJECT_ID}.svc.id.goog[${NAMESPACE}/${KSA_NAME}]"

# Annotate Kubernetes SA
kubectl annotate serviceaccount $KSA_NAME \
    --namespace=$NAMESPACE \
    iam.gke.io/gcp-service-account=${GSA_NAME}@${PROJECT_ID}.iam.gserviceaccount.com
```

#### Update Deployment

```yaml
# deployment.yaml
apiVersion: apps/v1
kind: Deployment
metadata:
  name: servo-worker
spec:
  template:
    metadata:
      labels:
        app: servo-worker
    spec:
      serviceAccountName: servo-worker  # Use the KSA
      containers:
      - name: servo-worker
        image: gcr.io/your-project/servo-worker:latest
        env:
        # Remove GOOGLE_APPLICATION_CREDENTIALS
        # Workload Identity provides credentials automatically
        - name: DATABASE_URL
          valueFrom:
            secretKeyRef:
              name: servo-secrets
              key: database-url
```

```bash
kubectl apply -f deployment.yaml
```

---

## 4. Remove JSON Key Files (CRITICAL)

Once Workload Identity is working:

### Delete Environment Variables

```bash
# For Cloud Run
gcloud run services update servo-worker \
    --remove-env-vars=GOOGLE_APPLICATION_CREDENTIALS \
    --region=us-central1

# For GKE - update deployment to remove env var
```

### Revoke Old JSON Keys

```bash
# List service account keys
gcloud iam service-accounts keys list \
    --iam-account=${GSA_NAME}@${PROJECT_ID}.iam.gserviceaccount.com

# Delete user-managed keys (keep Google-managed keys)
gcloud iam service-accounts keys delete KEY_ID \
    --iam-account=${GSA_NAME}@${PROJECT_ID}.iam.gserviceaccount.com
```

### Delete Secret Files from Repos/Storage

```bash
# Remove from Git history (if accidentally committed)
git filter-branch --force --index-filter \
  'git rm --cached --ignore-unmatch path/to/service-account-key.json' \
  --prune-empty --tag-name-filter cat -- --all

# Remove from container images
# Rebuild images without COPY commands for JSON keys
```

---

## 5. Verify Workload Identity

### Cloud Run Verification

```bash
# Deploy with Workload Identity
gcloud run deploy servo-worker \
    --image=gcr.io/${PROJECT_ID}/servo-worker:latest \
    --service-account=${GSA_NAME}@${PROJECT_ID}.iam.gserviceaccount.com \
    --region=us-central1

# Check logs for successful authentication
gcloud run logs read servo-worker --region=us-central1 --limit=50
```

### GKE Verification

```bash
# Exec into pod
kubectl exec -it <pod-name> -- /bin/sh

# Verify metadata server is accessible
curl -H "Metadata-Flavor: Google" \
  http://metadata.google.internal/computeMetadata/v1/instance/service-accounts/default/email

# Should return: servo-worker@your-project.iam.gserviceaccount.com
```

### Application-Level Verification

Servo's GCP auth module automatically detects Workload Identity:

```rust
// In servo-cloud-gcp/src/auth.rs
// Auto-detects environment and uses Workload Identity if available

let auth = GcpAuth::from_environment()?;  // No JSON key needed!
let token = auth.get_access_token(SCOPE).await?;
```

---

## Code Changes Required

### **None!** 🎉

Servo's `GcpAuth` already supports Workload Identity. The auth flow automatically falls back:

1. **Workload Identity** (if running on GKE/Cloud Run with WI enabled)
2. **Service Account JSON** (if `GOOGLE_APPLICATION_CREDENTIALS` is set)
3. **Application Default Credentials** (local development with `gcloud auth`)

Simply remove the `GOOGLE_APPLICATION_CREDENTIALS` environment variable and Workload Identity will be used.

---

## Local Development

For local development **without Workload Identity**:

### Option 1: Application Default Credentials (Recommended)

```bash
gcloud auth application-default login
```

This creates temporary credentials in `~/.config/gcloud/application_default_credentials.json`.

### Option 2: Service Account Impersonation

```bash
gcloud auth application-default login --impersonate-service-account=${GSA_NAME}@${PROJECT_ID}.iam.gserviceaccount.com
```

### Option 3: JSON Key (Least Secure - Dev Only)

```bash
export GOOGLE_APPLICATION_CREDENTIALS="/path/to/key.json"
```

**⚠️ NEVER commit JSON keys to version control!**

---

## Troubleshooting

### "Permission denied" errors

**Problem**: Service account lacks required permissions
**Solution**: Review IAM bindings

```bash
gcloud projects get-iam-policy $PROJECT_ID \
    --flatten="bindings[].members" \
    --filter="bindings.members:serviceAccount:${GSA_NAME}@${PROJECT_ID}.iam.gserviceaccount.com"
```

### "Failed to generate access token"

**Problem**: Workload Identity binding is incorrect
**Solution**: Verify binding between KSA and GSA

```bash
gcloud iam service-accounts get-iam-policy \
    ${GSA_NAME}@${PROJECT_ID}.iam.gserviceaccount.com
```

Look for `roles/iam.workloadIdentityUser` with your Kubernetes SA.

### "Metadata server not responding"

**Problem**: Workload Identity not enabled on cluster
**Solution**: Enable it

```bash
gcloud container clusters update $CLUSTER_NAME \
    --region=$REGION \
    --workload-pool=${PROJECT_ID}.svc.id.goog
```

---

## Security Best Practices

✅ **Principle of Least Privilege**: Grant only required roles
✅ **Separate Service Accounts**: Use different SAs for different workloads
✅ **Audit Regularly**: Review IAM bindings quarterly
✅ **Monitor Access**: Enable Cloud Audit Logs for service account usage
✅ **Rotate Nothing**: Workload Identity credentials auto-rotate (that's the point!)

---

## Migration Checklist

- [ ] Create Google Service Account
- [ ] Grant minimum required IAM roles
- [ ] Enable Workload Identity on GKE cluster (if using GKE)
- [ ] Configure Cloud Run with service account (if using Cloud Run)
- [ ] Create and bind Kubernetes Service Account (GKE only)
- [ ] Update deployment to use Workload Identity
- [ ] Test authentication in staging
- [ ] Remove `GOOGLE_APPLICATION_CREDENTIALS` environment variable
- [ ] Delete service account JSON keys
- [ ] Remove JSON key files from repos and container images
- [ ] Update documentation and runbooks
- [ ] Verify production deployment
- [ ] Monitor logs for auth errors

---

## References

- [Workload Identity Overview](https://cloud.google.com/kubernetes-engine/docs/concepts/workload-identity)
- [Cloud Run Service Identity](https://cloud.google.com/run/docs/securing/service-identity)
- [IAM Best Practices](https://cloud.google.com/iam/docs/best-practices)
- [Secret Manager](https://cloud.google.com/secret-manager/docs)
