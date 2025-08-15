# remove added folders
Remove-Item .\..\dataset\test\java_obfuscated\* -Recurse -Force
Remove-Item .\..\dataset\test\jsonl\* -Recurse -Force

Remove-Item .\..\dataset\train\java_obfuscated\* -Recurse -Force
Remove-Item .\..\dataset\train\jsonl\* -Recurse -Force

Remove-Item .\..\dataset\val\java_obfuscated\* -Recurse -Force
Remove-Item .\..\dataset\val\jsonl\* -Recurse -Force

