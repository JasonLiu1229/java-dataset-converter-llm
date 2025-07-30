# java-dataset-converter-llm

This package makes use of an external package called [proguard](https://github.com/Guardsquare/proguard?tab=readme-ov-file#-license) to obfuscate the java files.

After obfuscating the java files, these will be converted to .jsonl files, with the following layout:

```jsonl
{
    "prompt": "public class A { void b() { int x = 10; System.out.println(x); } }",
    "response": "public class Calculator { void printValue() { int value = 10; System.out.println(value); } }"
}
```

