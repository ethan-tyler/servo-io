//! Cloud Scheduler client for workflow scheduling
//!
//! Provides integration with GCP Cloud Scheduler for cron-based
//! workflow execution.

use crate::auth::GcpAuth;
use crate::{Error, Result};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Configuration for a scheduled workflow
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScheduleConfig {
    /// Cron expression (e.g., "0 2 * * *" for daily at 2 AM)
    pub cron: String,
    /// Timezone for cron interpretation (e.g., "America/New_York")
    pub timezone: String,
    /// Whether the schedule is enabled
    pub enabled: bool,
}

/// Cloud Scheduler job response
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SchedulerJobResponse {
    name: String,
    #[serde(default)]
    #[allow(dead_code)]
    state: Option<String>,
}

/// Cloud Scheduler manager for creating and managing scheduled jobs
pub struct CloudSchedulerManager {
    project_id: String,
    location: String,
    worker_url: String,
    auth: GcpAuth,
    http_client: reqwest::Client,
}

impl CloudSchedulerManager {
    /// Create a new Cloud Scheduler manager
    ///
    /// # Arguments
    ///
    /// * `project_id` - GCP project ID
    /// * `location` - GCP location/region (e.g., "us-central1")
    /// * `worker_url` - Cloud Run worker service URL
    /// * `service_account_json` - Service account credentials JSON
    pub fn new(
        project_id: String,
        location: String,
        worker_url: String,
        service_account_json: &str,
    ) -> Result<Self> {
        let auth = GcpAuth::from_service_account_json(service_account_json)?;

        let http_client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(30))
            .build()
            .map_err(|e| Error::Internal(format!("Failed to create HTTP client: {}", e)))?;

        Ok(Self {
            project_id,
            location,
            worker_url,
            auth,
            http_client,
        })
    }

    /// Create a Cloud Scheduler job for a workflow
    ///
    /// Creates an HTTP POST job that triggers workflow execution at the specified schedule.
    ///
    /// # Arguments
    ///
    /// * `workflow_id` - UUID of the workflow
    /// * `workflow_name` - Name of the workflow (used in job name)
    /// * `tenant_id` - Tenant identifier
    /// * `schedule` - Schedule configuration
    ///
    /// # Returns
    ///
    /// The Cloud Scheduler job name
    #[tracing::instrument(
        name = "cloud_scheduler.create_job",
        skip(self, schedule),
        fields(
            workflow_id = %workflow_id,
            workflow_name = %workflow_name,
            tenant_id = %tenant_id,
        )
    )]
    pub async fn create_job(
        &self,
        workflow_id: Uuid,
        workflow_name: &str,
        tenant_id: &str,
        schedule: &ScheduleConfig,
    ) -> Result<String> {
        let job_name = self.job_name(workflow_id, tenant_id);
        let target_url = format!(
            "{}/api/v1/workflows/{}/execute",
            self.worker_url, workflow_id
        );

        // Build job definition
        let job = serde_json::json!({
            "name": format!("{}/{}", self.parent_path(), job_name),
            "schedule": schedule.cron,
            "timeZone": schedule.timezone,
            "httpTarget": {
                "uri": target_url,
                "httpMethod": "POST",
                "headers": {
                    "Content-Type": "application/json",
                    "X-Servo-Tenant-Id": tenant_id,
                },
                "body": base64::Engine::encode(
                    &base64::engine::general_purpose::STANDARD,
                    serde_json::json!({
                        "workflow_id": workflow_id,
                        "tenant_id": tenant_id,
                        "triggered_by": "scheduler"
                    }).to_string()
                ),
                "oidcToken": {
                    "serviceAccountEmail": self.auth.service_account_email(),
                    "audience": &self.worker_url,
                }
            },
            "state": if schedule.enabled { "ENABLED" } else { "PAUSED" }
        });

        let api_url = format!(
            "https://cloudscheduler.googleapis.com/v1/{}/jobs",
            self.parent_path()
        );

        let access_token = self
            .auth
            .get_access_token("https://www.googleapis.com/auth/cloud-platform")
            .await?;

        let response = self
            .http_client
            .post(&api_url)
            .header("Authorization", format!("Bearer {}", access_token))
            .header("Content-Type", "application/json")
            .json(&job)
            .send()
            .await
            .map_err(|e| Error::Internal(format!("HTTP request failed: {}", e)))?;

        let status = response.status();
        if status.is_success() {
            let job_response: SchedulerJobResponse = response
                .json()
                .await
                .map_err(|e| Error::Api(format!("Failed to parse response: {}", e)))?;

            tracing::info!(
                job_name = %job_response.name,
                workflow_id = %workflow_id,
                "Cloud Scheduler job created"
            );

            Ok(job_response.name)
        } else {
            let body = response.text().await.unwrap_or_default();
            Err(Error::Api(format!(
                "Cloud Scheduler API returned {}: {}",
                status, body
            )))
        }
    }

    /// Update an existing Cloud Scheduler job
    ///
    /// # Arguments
    ///
    /// * `workflow_id` - UUID of the workflow
    /// * `tenant_id` - Tenant identifier
    /// * `schedule` - New schedule configuration
    #[tracing::instrument(
        name = "cloud_scheduler.update_job",
        skip(self, schedule),
        fields(
            workflow_id = %workflow_id,
            tenant_id = %tenant_id,
        )
    )]
    pub async fn update_job(
        &self,
        workflow_id: Uuid,
        tenant_id: &str,
        schedule: &ScheduleConfig,
    ) -> Result<String> {
        let job_name = self.job_name(workflow_id, tenant_id);
        let full_job_path = format!("{}/jobs/{}", self.parent_path(), job_name);
        let target_url = format!(
            "{}/api/v1/workflows/{}/execute",
            self.worker_url, workflow_id
        );

        let job = serde_json::json!({
            "schedule": schedule.cron,
            "timeZone": schedule.timezone,
            "httpTarget": {
                "uri": target_url,
                "httpMethod": "POST",
                "headers": {
                    "Content-Type": "application/json",
                    "X-Servo-Tenant-Id": tenant_id,
                },
                "body": base64::Engine::encode(
                    &base64::engine::general_purpose::STANDARD,
                    serde_json::json!({
                        "workflow_id": workflow_id,
                        "tenant_id": tenant_id,
                        "triggered_by": "scheduler"
                    }).to_string()
                ),
                "oidcToken": {
                    "serviceAccountEmail": self.auth.service_account_email(),
                    "audience": &self.worker_url,
                }
            },
            "state": if schedule.enabled { "ENABLED" } else { "PAUSED" }
        });

        let api_url = format!("https://cloudscheduler.googleapis.com/v1/{}", full_job_path);

        let access_token = self
            .auth
            .get_access_token("https://www.googleapis.com/auth/cloud-platform")
            .await?;

        let response = self
            .http_client
            .patch(&api_url)
            .header("Authorization", format!("Bearer {}", access_token))
            .header("Content-Type", "application/json")
            .query(&[("updateMask", "schedule,timeZone,state,httpTarget")])
            .json(&job)
            .send()
            .await
            .map_err(|e| Error::Internal(format!("HTTP request failed: {}", e)))?;

        let status = response.status();
        if status.is_success() {
            let job_response: SchedulerJobResponse = response
                .json()
                .await
                .map_err(|e| Error::Api(format!("Failed to parse response: {}", e)))?;

            tracing::info!(
                job_name = %job_response.name,
                workflow_id = %workflow_id,
                "Cloud Scheduler job updated"
            );

            Ok(job_response.name)
        } else {
            let body = response.text().await.unwrap_or_default();
            Err(Error::Api(format!(
                "Cloud Scheduler API returned {}: {}",
                status, body
            )))
        }
    }

    /// Delete a Cloud Scheduler job
    ///
    /// # Arguments
    ///
    /// * `workflow_id` - UUID of the workflow
    /// * `tenant_id` - Tenant identifier
    #[tracing::instrument(
        name = "cloud_scheduler.delete_job",
        skip(self),
        fields(
            workflow_id = %workflow_id,
            tenant_id = %tenant_id,
        )
    )]
    pub async fn delete_job(&self, workflow_id: Uuid, tenant_id: &str) -> Result<()> {
        let job_name = self.job_name(workflow_id, tenant_id);
        let full_job_path = format!("{}/jobs/{}", self.parent_path(), job_name);

        let api_url = format!("https://cloudscheduler.googleapis.com/v1/{}", full_job_path);

        let access_token = self
            .auth
            .get_access_token("https://www.googleapis.com/auth/cloud-platform")
            .await?;

        let response = self
            .http_client
            .delete(&api_url)
            .header("Authorization", format!("Bearer {}", access_token))
            .send()
            .await
            .map_err(|e| Error::Internal(format!("HTTP request failed: {}", e)))?;

        let status = response.status();
        if status.is_success() || status.as_u16() == 404 {
            // 404 is acceptable - job may have already been deleted
            tracing::info!(
                job_name = %job_name,
                workflow_id = %workflow_id,
                "Cloud Scheduler job deleted"
            );
            Ok(())
        } else {
            let body = response.text().await.unwrap_or_default();
            Err(Error::Api(format!(
                "Cloud Scheduler API returned {}: {}",
                status, body
            )))
        }
    }

    /// Pause a Cloud Scheduler job
    #[tracing::instrument(
        name = "cloud_scheduler.pause_job",
        skip(self),
        fields(
            workflow_id = %workflow_id,
            tenant_id = %tenant_id,
        )
    )]
    pub async fn pause_job(&self, workflow_id: Uuid, tenant_id: &str) -> Result<()> {
        let job_name = self.job_name(workflow_id, tenant_id);
        let full_job_path = format!("{}/jobs/{}", self.parent_path(), job_name);

        let api_url = format!(
            "https://cloudscheduler.googleapis.com/v1/{}:pause",
            full_job_path
        );

        let access_token = self
            .auth
            .get_access_token("https://www.googleapis.com/auth/cloud-platform")
            .await?;

        let response = self
            .http_client
            .post(&api_url)
            .header("Authorization", format!("Bearer {}", access_token))
            .header("Content-Type", "application/json")
            .body("{}")
            .send()
            .await
            .map_err(|e| Error::Internal(format!("HTTP request failed: {}", e)))?;

        let status = response.status();
        if status.is_success() {
            tracing::info!(
                job_name = %job_name,
                workflow_id = %workflow_id,
                "Cloud Scheduler job paused"
            );
            Ok(())
        } else {
            let body = response.text().await.unwrap_or_default();
            Err(Error::Api(format!(
                "Cloud Scheduler API returned {}: {}",
                status, body
            )))
        }
    }

    /// Resume a Cloud Scheduler job
    #[tracing::instrument(
        name = "cloud_scheduler.resume_job",
        skip(self),
        fields(
            workflow_id = %workflow_id,
            tenant_id = %tenant_id,
        )
    )]
    pub async fn resume_job(&self, workflow_id: Uuid, tenant_id: &str) -> Result<()> {
        let job_name = self.job_name(workflow_id, tenant_id);
        let full_job_path = format!("{}/jobs/{}", self.parent_path(), job_name);

        let api_url = format!(
            "https://cloudscheduler.googleapis.com/v1/{}:resume",
            full_job_path
        );

        let access_token = self
            .auth
            .get_access_token("https://www.googleapis.com/auth/cloud-platform")
            .await?;

        let response = self
            .http_client
            .post(&api_url)
            .header("Authorization", format!("Bearer {}", access_token))
            .header("Content-Type", "application/json")
            .body("{}")
            .send()
            .await
            .map_err(|e| Error::Internal(format!("HTTP request failed: {}", e)))?;

        let status = response.status();
        if status.is_success() {
            tracing::info!(
                job_name = %job_name,
                workflow_id = %workflow_id,
                "Cloud Scheduler job resumed"
            );
            Ok(())
        } else {
            let body = response.text().await.unwrap_or_default();
            Err(Error::Api(format!(
                "Cloud Scheduler API returned {}: {}",
                status, body
            )))
        }
    }

    /// Get job details
    pub async fn get_job(&self, workflow_id: Uuid, tenant_id: &str) -> Result<Option<String>> {
        let job_name = self.job_name(workflow_id, tenant_id);
        let full_job_path = format!("{}/jobs/{}", self.parent_path(), job_name);

        let api_url = format!("https://cloudscheduler.googleapis.com/v1/{}", full_job_path);

        let access_token = self
            .auth
            .get_access_token("https://www.googleapis.com/auth/cloud-platform")
            .await?;

        let response = self
            .http_client
            .get(&api_url)
            .header("Authorization", format!("Bearer {}", access_token))
            .send()
            .await
            .map_err(|e| Error::Internal(format!("HTTP request failed: {}", e)))?;

        let status = response.status();
        if status.is_success() {
            let job_response: SchedulerJobResponse = response
                .json()
                .await
                .map_err(|e| Error::Api(format!("Failed to parse response: {}", e)))?;
            Ok(Some(job_response.name))
        } else if status.as_u16() == 404 {
            Ok(None)
        } else {
            let body = response.text().await.unwrap_or_default();
            Err(Error::Api(format!(
                "Cloud Scheduler API returned {}: {}",
                status, body
            )))
        }
    }

    /// Generate a unique job name for a workflow
    fn job_name(&self, workflow_id: Uuid, tenant_id: &str) -> String {
        // Job names must match: [a-zA-Z0-9_-]{1,500}
        // Use tenant_id and workflow_id to ensure uniqueness
        format!(
            "servo-{}-{}",
            tenant_id.replace(['.', '/', '\\'], "-"),
            workflow_id.to_string().replace('-', "")
        )
    }

    /// Get the parent path for Cloud Scheduler jobs
    fn parent_path(&self) -> String {
        format!("projects/{}/locations/{}", self.project_id, self.location)
    }

    /// Create or update a job based on whether it exists
    ///
    /// This is a convenience method that handles the create-or-update pattern.
    pub async fn create_or_update_job(
        &self,
        workflow_id: Uuid,
        workflow_name: &str,
        tenant_id: &str,
        schedule: &ScheduleConfig,
    ) -> Result<String> {
        // Check if job exists
        match self.get_job(workflow_id, tenant_id).await? {
            Some(_) => {
                // Job exists, update it
                self.update_job(workflow_id, tenant_id, schedule).await
            }
            None => {
                // Job doesn't exist, create it
                self.create_job(workflow_id, workflow_name, tenant_id, schedule)
                    .await
            }
        }
    }
}
