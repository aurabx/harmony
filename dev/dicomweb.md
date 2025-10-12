# DICOMWeb testing

```zsh
curl -X GET \
  -H "Accept: application/dicom+json" \
  "https://127.0.0.1:8080/dicomweb/studies"

```

```zsh
curl -X GET \
  -H "Accept: application/dicom+json" \
  "https://127.0.0.1:8080/dicomweb/studies/1.2.840.113619.2.55.3.2831164357.781.1592405127.467"

```

```zsh
curl -X GET \
  -H "Accept: application/dicom+json" \
  "https://127.0.0.1:8080/dicomweb/studies/1.2.840.113619.2.55.3.2831164357.781.1592405127.467/series"

```

```zsh
curl -X GET \
  -H "Accept: application/dicom+json" \
  "https://127.0.0.1:8080/dicomweb/studies?PatientName=SMITH*"
```