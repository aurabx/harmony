# DICOMWeb testing

```zsh
curl -X GET \
  -H "Accept: application/dicom+json" \
  "http://127.0.0.1:8080/dicomweb/studies"

```

```zsh
curl -X GET \
  -H "Accept: application/dicom+json" \
  "http://127.0.0.1:8080/dicomweb/studies/1.3.6.1.4.1.5962.99.1.939772310.1977867020.1426868947350.4.0"

```

```zsh
curl -X GET \
  -H "Accept: application/dicom+json" \
  "http://127.0.0.1:8080/dicomweb/studies/1.3.6.1.4.1.5962.99.1.939772310.1977867020.1426868947350.4.0/series"

```

```zsh
curl -X GET \
  -H "Accept: application/dicom+json" \
  "http://127.0.0.1:8080/dicomweb/studies?PatientName=SMITH*"
```