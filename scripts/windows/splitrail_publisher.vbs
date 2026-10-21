' Run the splitrail publisher (`splitrail-dashboard.exe mqtt`) with no window, in this desktop
' session. Installed by install_windows_publisher.ps1 as the "splitrail-publisher" logon task.
Set shell = CreateObject("WScript.Shell")
here = CreateObject("Scripting.FileSystemObject").GetParentFolderName(WScript.ScriptFullName)
shell.Run """" & here & "\splitrail-dashboard.exe"" mqtt", 0, False
