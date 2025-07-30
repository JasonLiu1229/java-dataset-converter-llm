# java-dataset-converter-llm

This package makes use of an external package called [proguard](https://github.com/Guardsquare/proguard?tab=readme-ov-file#-license) to obfuscate the java files.
Version that will be used for this project is v7.7. The binary is not provided in the project and thus have to be downloaded manually. Make sure that this folder is found in the `src/tools` folder.

We also make use of a decompiler because proguard makes use of jar files and not just java. So to do this, we also make use of [cfr](https://www.benf.org/other/cfr/). Make sure to download this too and add it too the `src/tools` folder. 

After obfuscating the java files, these will be converted to .jsonl files, with the following layout:

```jsonl
{
    "prompt": "public class A { void b() { int x = 10; System.out.println(x); } }",
    "response": "public class Calculator { void printValue() { int value = 10; System.out.println(value); } }"
}
```
