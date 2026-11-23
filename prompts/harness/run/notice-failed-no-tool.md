{{agent}} failed twice without calling a tool ({{task}})
Two runs of task {{task}} ended with `STATUS: FAILED` and neither called a tool step of the bus, so no work was done:
the {{cli}} CLI did not offer the tools of the bus, or the {{model}} model that runs in it did not use them.

What it said last:

{{message}}

Do not assign this task to another agent until the {{cli}} CLI setup and the {{model}} model configuration are checked:
a run that cannot call the tools of the bus cannot do the work. Once the configuration is fixed, resume the task and it
will make a clean start.