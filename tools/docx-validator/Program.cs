using DocumentFormat.OpenXml;
using DocumentFormat.OpenXml.Packaging;
using DocumentFormat.OpenXml.Validation;
using DocumentFormat.OpenXml.Wordprocessing;
using System.Text.Json;

if (args is ["--self-test"])
{
    foreach (var invalid in new[] { false, true })
    {
        using var stream = new MemoryStream();
        using (var created = WordprocessingDocument.Create(stream, WordprocessingDocumentType.Document, true))
        {
            var main = created.AddMainDocumentPart();
            // Both documents are well-formed XML. A run directly inside body
            // violates WordprocessingML's content model; XML parsing misses it.
            var body = new Body();
            if (invalid) body.AppendChild(new Run(new Text("invalid")));
            else body.AppendChild(new Paragraph(new Run(new Text("valid"))));
            main.Document = new Document(body);
        }
        stream.Position = 0;
        using var reopened = WordprocessingDocument.Open(stream, false);
        var count = new OpenXmlValidator(FileFormatVersions.Microsoft365).Validate(reopened).Count();
        if ((count > 0) != invalid)
            throw new InvalidOperationException($"Schema oracle self-test failed: invalid={invalid}, errors={count}");
    }
    Console.Error.WriteLine("OOXML schema oracle self-test passed (valid accepted, well-formed schema violation rejected).");
    return 0;
}

if (args.Length == 0)
{
    Console.Error.WriteLine("Usage: DocxValidator <docx-file-or-directory> [...]");
    return 2;
}
var validator = new OpenXmlValidator(FileFormatVersions.Microsoft365) { MaxNumberOfErrors = 0 };
var files = args.SelectMany(path => Directory.Exists(path)
    ? Directory.EnumerateFiles(path, "*.docx", SearchOption.AllDirectories)
    : new[] { path }).Distinct().Order().ToArray();
if (files.Length == 0)
{
    Console.Error.WriteLine("No DOCX files found; refusing an empty validation run.");
    return 2;
}
int failed = 0, errors = 0;
foreach (var file in files)
{
    try
    {
        using var document = WordprocessingDocument.Open(file, false);
        var findings = validator.Validate(document).ToArray();
        if (findings.Length == 0) continue;
        failed++;
        errors += findings.Length;
        foreach (var error in findings)
            Console.WriteLine(JsonSerializer.Serialize(new {
                file, id = error.Id, type = error.ErrorType.ToString(),
                part = error.Part?.Uri.ToString(), path = error.Path?.XPath,
                description = error.Description
            }));
    }
    catch (Exception exception)
    {
        failed++;
        errors++;
        Console.WriteLine(JsonSerializer.Serialize(new { file, exception = exception.ToString() }));
    }
}
Console.Error.WriteLine($"Microsoft365 OOXML validation: {files.Length} files, {failed} invalid, {errors} errors.");
return failed == 0 ? 0 : 1;
