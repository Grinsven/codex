@@ -35,6 +35,7 @@ pub(crate) struct ToolsConfig {
     pub apply_patch_tool_type: Option<ApplyPatchToolType>,
     pub web_search_request: bool,
     pub include_view_image_tool: bool,
+    pub include_agent_tool: bool,
     pub experimental_supported_tools: Vec<String>,
 }
 
@@ -78,6 +79,7 @@ impl ToolsConfig {
             apply_patch_tool_type,
             web_search_request: include_web_search_request,
             include_view_image_tool,
+            include_agent_tool: true,
             experimental_supported_tools: model_family.experimental_supported_tools.clone(),
         }
     }
@@ -219,6 +221,45 @@ fn create_exec_command_tool() -> ToolSpec {
     })
 }
 
+fn create_agent_tool() -> ToolSpec {
+    let mut properties = BTreeMap::new();
+    properties.insert(
+        "name".to_string(),
+        JsonSchema::String {
+            description: Some("Registered agent name to run.".to_string()),
+        },
+    );
+    properties.insert(
+        "task".to_string(),
+        JsonSchema::String {
+            description: Some("Task or instruction for the agent to complete.".to_string()),
+        },
+    );
+    properties.insert(
+        "context".to_string(),
+        JsonSchema::String {
+            description: Some("Optional additional context the agent should consider.".to_string()),
+        },
+    );
+    properties.insert(
+        "plan_item_id".to_string(),
+        JsonSchema::String {
+            description: Some("Optional plan item id associated with this run.".to_string()),
+        },
+    );
+
+    ToolSpec::Function(ResponsesApiTool {
+        name: "agent".to_string(),
+        description: "Invoke a named agent with a task to execute.".to_string(),
+        strict: false,
+        parameters: JsonSchema::Object {
+            properties,
+            required: Some(vec!["name".to_string(), "task".to_string()]),
+            additional_properties: Some(false.into()),
+        },
+    })
+}
+
 fn create_write_stdin_tool() -> ToolSpec {
     let mut properties = BTreeMap::new();
     properties.insert(
@@ -973,6 +1014,7 @@ pub(crate) fn build_specs(
     config: &ToolsConfig,
     mcp_tools: Option<HashMap<String, mcp_types::Tool>>,\n ) -> ToolRegistryBuilder {
+    use crate::tools::handlers::AgentHandler;
     use crate::tools::handlers::ApplyPatchHandler;
     use crate::tools::handlers::GrepFilesHandler;
     use crate::tools::handlers::ListDirHandler;
@@ -993,6 +1035,7 @@ pub(crate) fn build_specs(
     let unified_exec_handler = Arc::new(UnifiedExecHandler);
     let plan_handler = Arc::new(PlanHandler);
     let apply_patch_handler = Arc::new(ApplyPatchHandler);
+    let agent_handler = Arc::new(AgentHandler);
     let view_image_handler = Arc::new(ViewImageHandler);
     let mcp_handler = Arc::new(McpHandler);
     let mcp_resource_handler = Arc::new(McpResourceHandler);
@@ -1095,6 +1138,11 @@ pub(crate) fn build_specs(
         builder.register_handler("view_image", view_image_handler);
     }
 
+    if config.include_agent_tool {
+        builder.push_spec_with_parallel_support(create_agent_tool(), true);
+        builder.register_handler("agent", agent_handler);
+    }
+
     if let Some(mcp_tools) = mcp_tools {
         let mut entries: Vec<(String, mcp_types::Tool)> = mcp_tools.into_iter().collect();
         entries.sort_by(|a, b| a.0.cmp(&b.0));
@@ -1253,6 +1301,7 @@ mod tests {
             create_apply_patch_freeform_tool(),
             ToolSpec::WebSearch {},
             create_view_image_tool(),
+            create_agent_tool(),
         ] {
             expected.insert(tool_name(&spec).to_string(), spec);
         }
@@ -1297,6 +1346,7 @@ mod tests {
                 "update_plan",
                 "apply_patch",
                 "view_image",
+                "agent",
             ],
         );
     }
@@ -1314,6 +1364,7 @@ mod tests {
                 "update_plan",
                 "apply_patch",
                 "view_image",
+                "agent",
             ],
         );
     }
@@ -1335,6 +1386,7 @@ mod tests {
                 "apply_patch",
                 "web_search",
                 "view_image",
+                "agent",
             ],
         );
     }
@@ -1356,6 +1408,7 @@ mod tests {
                 "apply_patch",
                 "web_search",\n                 "view_image",
+                "agent",
             ],
         );
     }
@@ -1372,6 +1425,7 @@ mod tests {
                 "read_mcp_resource",
                 "update_plan",
                 "view_image",
+                "agent",
             ],
         );
     }
@@ -1389,6 +1443,7 @@ mod tests {
                 "update_plan",
                 "apply_patch",
                 "view_image",
+                "agent",
             ],
         );
     }
@@ -1405,6 +1460,7 @@ mod tests {
                 "read_mcp_resource",
                 "update_plan",
                 "view_image",
+                "agent",
             ],
         );
     }
@@ -1422,6 +1478,7 @@ mod tests {
                 "update_plan",
                 "apply_patch",
                 "view_image",
+                "agent",
             ],
         );
     }
@@ -1440,6 +1497,7 @@ mod tests {
                 "update_plan",
                 "apply_patch",
                 "view_image",
+                "agent",
             ],
         );
     }
@@ -1460,6 +1518,7 @@ mod tests {
                 "update_plan",
                 "web_search",
                 "view_image",
+                "agent",
             ],
         );
     }