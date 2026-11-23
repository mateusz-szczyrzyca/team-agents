{{agent}} failed twice without calling a tool ({{task}})
Two runs of task {{task}} ended with `STATUS: FAILED` and neither called a tool step of the bus, so no work was done:
the {{cli}} CLI did not offer the tools of the bus, or the {{model}} model that runs in it did not use them.

What it said last:

{{message}}

Do not give {{agent}} more work until its {{cli}} CLI setup and {{model}} model configuration are checked.
